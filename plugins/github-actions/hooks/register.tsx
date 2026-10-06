import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'
import { disparaRun, ehGithub, jobs, pr, precisaConsultar, runsAtuais, situacao } from './gh'
import type { GhView, Job, JobGh, PrGh, RunGh, Workflow } from './gh'
import { desenharFaixa } from './faixa'

const view = atom({ plugin: 'github-actions', key: 'view' } as const, null as GhView | null)

const INTERVALO_MS = 15_000
// Depois de um push o run novo leva alguns segundos para aparecer na API.
const ESPERA_PUSH_MS = 3_000
const PRAZO_PUSH_MS = 120_000
// Sem nada rodando, o fim de turno reconsulta no máximo uma vez por minuto.
const REVISITA_MS = 60_000

type Timer = { cancel: () => void }

let timer: Timer | null = null
let consultando = false
let ultima = 0
let cabeca = ''
// Commit empurrado cujo run ainda não apareceu: segue consultando até ele surgir ou o prazo vencer.
let aguardando: { sha: string; ate: number } | null = null
// Run terminado não muda mais: os jobs dele ficam guardados.
const jobsFinais = new Map<number, Job[]>()

async function sh($: EngineInterface, argv: string[]): Promise<string | null> {
  try {
    const r = await $.process.run(argv, { timeoutMs: 20_000 })
    return r.exitCode === 0 ? r.stdout.trim() : null
  } catch {
    return null
  }
}

async function json<T>($: EngineInterface, argv: string[]): Promise<T | null> {
  const out = await sh($, argv)
  if (out === null) return null
  try {
    return JSON.parse(out) as T
  } catch {
    return null
  }
}

async function workflowDe($: EngineInterface, r: RunGh): Promise<Workflow> {
  const base = { id: r.databaseId, nome: r.workflowName, url: r.url }
  const fim = r.status === 'completed' ? situacao(r.status, r.conclusion) : null
  const guardados = jobsFinais.get(r.databaseId)
  if (fim && guardados) return { ...base, situacao: fim, jobs: guardados }
  const v = await json<{ jobs: JobGh[] }>($, ['gh', 'run', 'view', String(r.databaseId), '--json', 'jobs'])
  const js = jobs(v?.jobs ?? [])
  if (fim && v) jobsFinais.set(r.databaseId, js)
  return { ...base, situacao: fim ?? (js.some(j => j.situacao === 'rodando') ? 'rodando' : 'esperando'), jobs: js }
}

async function consultar($: EngineInterface): Promise<GhView | null> {
  const remoto = await sh($, ['git', 'remote', 'get-url', 'origin'])
  const branch = await sh($, ['git', 'branch', '--show-current'])
  if (!remoto || !branch || !ehGithub(remoto)) return null
  const [runs, prGh] = await Promise.all([
    json<RunGh[]>($, ['gh', 'run', 'list', '--branch', branch, '--limit', '30',
      '--json', 'databaseId,workflowName,status,conclusion,headSha,url']),
    json<PrGh>($, ['gh', 'pr', 'view', branch,
      '--json', 'number,title,url,state,isDraft,reviewDecision,statusCheckRollup']),
  ])
  if (runs === null) return null
  const atuais = runsAtuais(runs)
  if (aguardando && (atuais[0]?.headSha === aguardando.sha || (await $.clock.now()) > aguardando.ate)) aguardando = null
  const workflows = await Promise.all(atuais.map(r => workflowDe($, r)))
  return { branch, workflows, pr: prGh ? pr(prGh) : null }
}

async function atualizar($: EngineInterface): Promise<void> {
  if (consultando) return
  consultando = true
  timer?.cancel()
  timer = null
  try {
    const v = await consultar($)
    ultima = await $.clock.now()
    await update($, view, () => v)
    if (v && (precisaConsultar(v) || aguardando)) timer = $.clock.after(INTERVALO_MS, () => void atualizar($))
  } finally {
    consultando = false
  }
}

async function headAtual($: EngineInterface): Promise<string> {
  return `${await sh($, ['git', 'branch', '--show-current'])}@${await sh($, ['git', 'rev-parse', 'HEAD'])}`
}

async function abrir($: EngineInterface, url: string): Promise<void> {
  // O plugin do Hangar reconhece estes abridores e, no clique vindo do app, abre no aparelho.
  for (const argv of [['xdg-open', url], ['open', url], ['cmd', '/c', 'start', '', url]]) {
    try {
      if ((await $.process.run(argv, { timeoutMs: 10_000 })).exitCode === 0) return
    } catch {
      // abridor ausente nesta plataforma: tenta o próximo
    }
  }
  $.ui.toast(`Não consegui abrir ${url}`)
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const r = await next(e)
    cabeca = await headAtual($)
    void atualizar($)
    return r
  })

  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    const r = await next(e)
    if (disparaRun(e.command)) {
      const sha = await sh($, ['git', 'rev-parse', 'HEAD'])
      if (sha) aguardando = { sha, ate: (await $.clock.now()) + PRAZO_PUSH_MS }
      timer?.cancel()
      timer = $.clock.after(ESPERA_PUSH_MS, () => void atualizar($))
    }
    return r
    // Só observa: falha aqui nunca segura o comando.
  }).catch(($, e, next) => next(e))

  // Troca de branch ou commit novo reconsulta; fora isso, uma revisita por minuto no máximo.
  on('turn.complete', async ($, e, next) => {
    const r = await next(e)
    if (!timer && !consultando) {
      const h = await headAtual($)
      if (h !== cabeca || (await $.clock.now()) - ultima > REVISITA_MS) {
        cabeca = h
        void atualizar($)
      }
    }
    return r
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    const v = e.props.hasSurvey ? null : await read($, view)
    if (!v || (v.workflows.length === 0 && !v.pr)) return next(e)
    const t = $.ui.resolve(e)
    const nosso = desenharFaixa(t, v, e.props.bodyColumns, url => void abrir($, url))
    const abaixo = await next(e).catch(() => null)
    if (!abaixo) return nosso
    const { Box } = t
    return <Box flexDirection="column">{nosso}{abaixo}</Box>
  })
}
