import { describe, expect, test } from 'claude-code/testing'
import { checks, classificarFalha, piorErro, textoAviso, disparaRun, ehGithub, ehPush, jobs, lembrarCommit, rotulosAbrir, precisaConsultar, runsVisiveis, situacao } from './gh'
import type { GhView, RunGh } from './gh'
import { rotuloJob } from './faixa'

const run = (id: number, wf: string, sha: string, status = 'completed', conclusion = 'success'): RunGh =>
  ({ databaseId: id, workflowName: wf, status, conclusion, headSha: sha, url: `u/${id}` })

describe('gh', () => {
  test('situação do run e do job', () => {
    expect(situacao('in_progress', '')).toBe('rodando')
    expect(situacao('queued', '')).toBe('esperando')
    expect(situacao('completed', 'success')).toBe('ok')
    expect(situacao('COMPLETED', 'FAILURE')).toBe('falhou')
    expect(situacao('completed', 'timed_out')).toBe('falhou')
    expect(situacao('completed', 'skipped')).toBe('pulado')
  })

  test('último run de cada workflow, mais os de commit velho que ainda rodam', () => {
    const r = runsVisiveis([
      [run(5, 'CI', 'b'), run(4, 'Server', 'b', 'queued', '')],
      [run(3, 'Server', 'a', 'in_progress', ''), run(2, 'CI', 'a'), run(1, 'Native', 'a')],
    ])
    // CI velho terminado sai; Server velho rodando fica; Native só existe no velho e fica.
    expect(r.map(x => x.databaseId)).toEqual([5, 4, 3, 1])
  })

  test('commits lembrados: mais novo primeiro, sem repetir, com teto', () => {
    const c = (sha: string) => ({ sha, branch: 'x' })
    expect(lembrarCommit([c('b'), c('a')], c('c'), 2)).toEqual([c('c'), c('b')])
    expect(lembrarCommit([c('b'), c('a')], c('a'), 5)).toEqual([c('a'), c('b')])
  })

  test('gh sem login vira aviso na faixa', () => {
    const msg = 'gh run list --commit: To get started with GitHub CLI, please run:  gh auth login'
    expect(classificarFalha(msg)).toBe('login')
    expect(classificarFalha('failed to get runs: HTTP 401: Bad credentials (https://api.github.com/...)')).toBe('login')
    expect(textoAviso('login', msg, null)).toBe('gh sem login · rode gh auth login')
  })

  test('limite da API vira aviso com o horário de volta', () => {
    const msg = 'gh run list --commit: HTTP 403: API rate limit exceeded for user ID 123.'
    expect(classificarFalha(msg)).toBe('limite')
    expect(classificarFalha('GraphQL: API rate limit exceeded for user ID 123.')).toBe('limite')
    expect(textoAviso('limite', msg, '21:40')).toBe('limite da API até 21:40')
    expect(textoAviso('limite', msg, null)).toBe('limite da API do GitHub')
  })

  test('de vários erros, vale o mais grave', () => {
    const outra = 'dial tcp: timeout'
    const limite = 'HTTP 403: API rate limit exceeded'
    const login = 'HTTP 401: Bad credentials'
    expect(piorErro(null, outra)).toBe(outra)
    expect(piorErro(outra, limite)).toBe(limite)
    expect(piorErro(limite, login)).toBe(limite)
    expect(piorErro(outra, login)).toBe(login)
  })

  test('outra falha mostra a primeira linha do erro', () => {
    expect(classificarFalha('dial tcp: lookup api.github.com: no such host')).toBe('outra')
    expect(textoAviso('outra', 'Error: dial tcp: no such host\nmais', null)).toBe('gh falhou: dial tcp: no such host')
  })

  test('só git push registra commit', () => {
    expect(ehPush('git push -u origin x')).toBe(true)
    expect(ehPush('gh pr view 3')).toBe(false)
  })

  test('rótulo do botão nunca se repete', () => {
    const w = (id: number, nome: string, sha: string) => ({ id, nome, sha })
    expect(rotulosAbrir([w(1, 'CI', 'aaaaaaaa'), w(2, 'Server', 'aaaaaaaa')])).toEqual(['abrir CI', 'abrir Server'])
    expect(rotulosAbrir([w(1, 'CI', 'aaaaaaaa'), w(2, 'CI', 'bbbbbbbb')])).toEqual(['abrir CI aaaaaaa', 'abrir CI bbbbbbb'])
    // push e pull_request do mesmo commit: o id do run desempata
    expect(rotulosAbrir([w(1, 'CI', 'aaaaaaaa'), w(2, 'CI', 'aaaaaaaa')])).toEqual(['abrir CI #1', 'abrir CI #2'])
  })

  test('job rodando mostra a etapa atual; job que falhou guarda o passo', () => {
    const [rodando, falhou] = jobs([
      { name: 'test', status: 'in_progress', conclusion: '', steps: [
        { name: 'checkout', status: 'completed', conclusion: 'success' },
        { name: 'pytest', status: 'in_progress', conclusion: '' },
      ] },
      { name: 'build', status: 'completed', conclusion: 'failure', steps: [
        { name: 'setup', status: 'completed', conclusion: 'success' },
        { name: 'Compile', status: 'completed', conclusion: 'failure' },
      ] },
    ])
    if (!rodando || !falhou) throw new Error('jobs perdidos')
    expect(rotuloJob(rodando)).toBe('● test · pytest')
    expect(falhou.passo).toBe('Compile')
    expect(rotuloJob(falhou)).toBe('✕ build')
  })

  test('resumo dos checks do PR', () => {
    expect(checks([
      { __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'SUCCESS' },
      { __typename: 'CheckRun', status: 'IN_PROGRESS', conclusion: null },
      { __typename: 'CheckRun', status: 'COMPLETED', conclusion: 'FAILURE' },
      { __typename: 'StatusContext', state: 'PENDING' },
    ])).toEqual({ ok: 1, falhou: 1, rodando: 2 })
  })

  test('consulta segue só enquanto algo roda', () => {
    const v = (s: 'ok' | 'rodando'): GhView => ({ branch: 'x', pr: null, workflows: [
      { id: 1, nome: 'CI', sha: 'x', situacao: s, url: '', jobs: [{ nome: 'a', situacao: s, passo: null }] },
    ] })
    expect(precisaConsultar(v('rodando'))).toBe(true)
    expect(precisaConsultar(v('ok'))).toBe(false)
    expect(precisaConsultar(null)).toBe(false)
  })

  test('comandos que disparam run e remoto do GitHub', () => {
    expect(disparaRun('git push -u origin x')).toBe(true)
    expect(disparaRun('gh pr create')).toBe(true)
    expect(disparaRun('git status')).toBe(false)
    expect(ehGithub('https://github.com/jeffer1312/hangar')).toBe(true)
    expect(ehGithub('git@github.com:a/b.git')).toBe(true)
    expect(ehGithub('https://gitlab.example.com/a/b.git')).toBe(false)
  })
})
