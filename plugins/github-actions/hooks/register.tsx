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
let refazer = false
let ultima = 0
let cabeca = ''
// Commit empurrado cujo run ainda não apareceu: segue consultando até ele surgir ou o prazo vencer.
let aguardando: { sha: string; ate: number } | null = null
// Run terminado não muda mais: os jobs dele ficam guardados (re-run volta a in_progress e limpa).
const jobsFinais = new Map<number, Job[]>()

type Saida = { ok: boolean; out: string; err: string }

// null: o comando nem existe aqui (sem git ou sem gh), o que esconde a faixa sem ser erro.
async function sh($: EngineInterface, argv: string[]): Promise<Saida | null> {
  try {
    const r = await $.process.run(argv, { timeoutMs: 20_000 })
    return { ok: r.exitCode === 0, out: r.stdout.trim(), err: r.stderr.trim() }
  } catch {
    return null
  }
}

async function texto($: EngineInterface, argv: string[]): Promise<string | null> {
  const r = await sh($, argv)
  return r?.ok ? r.out : null
}

// Falha do gh vira exceção com o motivo: quem chama mantém a última leitura em vez de apagá-la.
function lerJson<T>(r: Saida | null, argv: string[]): T {
  if (!r) throw new Error(`${argv[0]} indisponível`)
  if (!r.ok) throw new Error(`${argv.slice(0, 3).join(' ')}: ${r.err || 'falhou'}`)
  return JSON.parse(r.out) as T
}

async function workflowDe($: EngineInterface, r: RunGh, antes: Workflow | undefined): Promise<Workflow> {
  const base = { id: r.databaseId, nome: r.workflowName, url: r.url }
  const fim = r.status === 'completed' ? situacao(r.status, r.conclusion) : null
  if (!fim) jobsFinais.delete(r.databaseId)
  const guardados = jobsFinais.get(r.databaseId)
  if (fim && guardados) return { ...base, situacao: fim, jobs: guardados }
  const argv = ['gh', 'run', 'view', String(r.databaseId), '--json', 'jobs']
  let js: Job[]
  try {
    js = jobs(lerJson<{ jobs: JobGh[] }>(await sh($, argv), argv).jobs)
    if (fim) jobsFinais.set(r.databaseId, js)
  } catch (err) {
    $.ui.log(`github-actions: ${String(err)}`, { to: 'debug' })
    js = antes?.id === r.databaseId ? antes.jobs : []
  }
  return { ...base, situacao: fim ?? (js.some(j => j.situacao === 'rodando') ? 'rodando' : 'esperando'), jobs: js }
}

async function consultar($: EngineInterface, antes: GhView | null): Promise<GhView | null> {
  const remoto = await texto($, ['git', 'remote', 'get-url', 'origin'])
  const branch = await texto($, ['git', 'branch', '--show-current'])
  if (!remoto || !branch || !ehGithub(remoto)) return null
  const runArgv = ['gh', 'run', 'list', '--branch', branch, '--limit', '30',
    '--json', 'databaseId,workflowName,status,conclusion,headSha,url']
  const prArgv = ['gh', 'pr', 'view', branch,
    '--json', 'number,title,url,state,isDraft,reviewDecision,statusCheckRollup']
  const [runSaida, prSaida] = await Promise.all([sh($, runArgv), sh($, prArgv)])
  if (!runSaida) return null
  const atuais = runsAtuais(lerJson<RunGh[]>(runSaida, runArgv))
  if (aguardando && atuais[0]?.headSha === aguardando.sha) aguardando = null
  const mesma = antes?.branch === branch ? antes : null
  const workflows = await Promise.all(atuais.map(r => workflowDe($, r, mesma?.workflows.find(w => w.nome === r.workflowName))))
  let p = mesma?.pr ?? null
  if (prSaida?.ok) p = pr(JSON.parse(prSaida.out) as PrGh)
  else if (!prSaida || /no pull requests found/i.test(prSaida.err)) p = null
  else $.ui.log(`github-actions: gh pr view: ${prSaida.err}`, { to: 'debug' })
  return { branch, workflows, pr: p }
}

function agendar($: EngineInterface, ms: number): void {
  timer?.cancel()
  timer = $.clock.after(ms, () => void atualizar($))
}

async function atualizar($: EngineInterface): Promise<void> {
  timer?.cancel()
  timer = null
  // Pedido que chega com outra consulta em curso não se perde: ela refaz ao terminar.
  if (consultando) {
    refazer = true
    return
  }
  consultando = true
  let v: GhView | null = null
  try {
    v = await read($, view)
    v = await consultar($, v)
    await update($, view, () => v)
  } catch (err) {
    // gh fora do ar, sem login ou com limite: a faixa fica como estava e tenta de novo.
    $.ui.log(`github-actions: ${String(err)}`, { to: 'debug' })
  } finally {
    consultando = false
    ultima = await $.clock.now().catch(() => ultima)
    if (aguardando && ultima > aguardando.ate) aguardando = null
    if (refazer) {
      refazer = false
      agendar($, 0)
    } else if (aguardando || precisaConsultar(v)) {
      agendar($, INTERVALO_MS)
    }
  }
}

async function headAtual($: EngineInterface): Promise<string> {
  return `${await texto($, ['git', 'branch', '--show-current'])}@${await texto($, ['git', 'rev-parse', 'HEAD'])}`
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
      const sha = await texto($, ['git', 'rev-parse', 'HEAD'])
      if (sha) aguardando = { sha, ate: (await $.clock.now()) + PRAZO_PUSH_MS }
      agendar($, ESPERA_PUSH_MS)
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
