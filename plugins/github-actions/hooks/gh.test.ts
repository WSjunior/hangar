import { describe, expect, test } from 'claude-code/testing'
import { checks, disparaRun, ehGithub, jobs, precisaConsultar, runsAtuais, situacao } from './gh'
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

  test('só o último run de cada workflow do commit mais novo', () => {
    const r = runsAtuais([run(3, 'CI', 'b'), run(2, 'Native', 'b'), run(1, 'CI', 'b'), run(0, 'Docs', 'a')])
    expect(r.map(x => x.databaseId)).toEqual([3, 2])
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
      { id: 1, nome: 'CI', situacao: s, url: '', jobs: [{ nome: 'a', situacao: s, passo: null }] },
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
