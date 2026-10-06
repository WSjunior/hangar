import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'
import { disparaRun, ehGithub, ehPush, emAndamento, jobs, lembrarCommit, pr, precisaConsultar, runsVisiveis, situacao } from './gh'
import type { Empurrado, Situacao } from './gh'
import type { GhView, Job, JobGh, PrGh, RunGh, Workflow } from './gh'
import { desenharFaixa } from './faixa'

const view = atom({ plugin: 'github-actions', key: 'view' } as const, null as GhView | null)

const INTERVALO_MS = 15_000
// Depois de um push o run novo leva alguns segundos para aparecer na API.
const ESPERA_PUSH_MS = 3_000
const PRAZO_PUSH_MS = 120_000
// Sem nada rodando, o fim de turno reconsulta no máximo uma vez por minuto.
const REVISITA_MS = 60_000
// Commits empurrados lembrados por sessão, e quantas sessões ficam guardadas.
const MAX_COMMITS = 5
const MAX_SESSOES = 30
const SALVO = 'commits:'

type Salvo = { em: number; commits: Empurrado[] }

// Volta da situação ao par do gh, para remontar o run de um commit que não respondeu.
const CONCLUSAO: Record<Situacao, string> = {
  ok: 'success', falhou: 'failure', pulado: 'skipped', cancelado: 'cancelled', rodando: '', esperando: '',
}

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
  const base = { id: r.databaseId, nome: r.workflowName, sha: r.headSha, url: r.url }
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

async function commitsDaSessao($: EngineInterface): Promise<Empurrado[]> {
  const salvo = (await $.store.get(SALVO + (await $.session.id()))) as Salvo | undefined
  return Array.isArray(salvo?.commits) ? salvo.commits : []
}

async function gravar($: EngineInterface, commits: Empurrado[]): Promise<void> {
  const salvo: Salvo = { em: await $.clock.now(), commits }
  await $.store.set(SALVO + (await $.session.id()), salvo)
}

// Em fila: dois pushes seguidos não leem a mesma lista e perdem um commit.
let gravando: Promise<void> = Promise.resolve()
function guardarCommit($: EngineInterface, novo: Empurrado): Promise<void> {
  gravando = gravando.then(async () => gravar($, lembrarCommit(await commitsDaSessao($), novo, MAX_COMMITS)))
  return gravando
}

// Cada sessão grava uma chave; as mais velhas saem uma vez, no início, nunca a desta sessão.
async function podar($: EngineInterface): Promise<void> {
  const minha = SALVO + (await $.session.id())
  const atuais = await commitsDaSessao($)
  if (atuais.length) await gravar($, atuais)
  const chaves = (await $.store.keys()).filter(k => k.startsWith(SALVO) && k !== minha)
  if (chaves.length < MAX_SESSOES) return
  const datas = await Promise.all(chaves.map(async k => ({ k, em: ((await $.store.get(k)) as Salvo | undefined)?.em ?? 0 })))
  datas.sort((a, b) => a.em - b.em)
  await Promise.all(datas.slice(0, chaves.length - MAX_SESSOES + 1).map(({ k }) => $.store.delete(k)))
}

// Só os runs dos commits que ESTA sessão empurrou; o PR é o da branch.
async function consultar($: EngineInterface, antes: GhView | null): Promise<GhView | null> {
  const remoto = await texto($, ['git', 'remote', 'get-url', 'origin'])
  const branch = await texto($, ['git', 'branch', '--show-current'])
  if (!remoto || !branch || !ehGithub(remoto)) return null
  // Só os desta branch: trocou de branch, os runs da outra não se misturam ao PR desta.
  const shas = (await commitsDaSessao($)).filter(c => c.branch === branch).map(c => c.sha)
  const prArgv = ['gh', 'pr', 'view', branch,
    '--json', 'number,title,url,state,isDraft,reviewDecision,statusCheckRollup']
  const [prSaida, ...lidos] = await Promise.all([
    sh($, prArgv),
    ...shas.map(async sha => {
      const argv = ['gh', 'run', 'list', '--commit', sha, '--limit', '20',
        '--json', 'databaseId,workflowName,status,conclusion,headSha,url']
      return { sha, saida: await sh($, argv), argv }
    }),
  ])
  // Commit que falhou fica com os runs da leitura anterior; todos falhando, a leitura inteira fica.
  const porCommit: RunGh[][] = []
  let falhas = 0
  for (const { sha, saida, argv } of lidos) {
    try {
      porCommit.push(lerJson<RunGh[]>(saida, argv))
    } catch (err) {
      falhas++
      $.ui.log(`github-actions: ${String(err)}`, { to: 'debug' })
      porCommit.push((antes?.workflows ?? []).filter(w => w.sha === sha).map(w => ({
        databaseId: w.id, workflowName: w.nome, headSha: w.sha, url: w.url,
        status: emAndamento(w.situacao) ? 'in_progress' : 'completed', conclusion: CONCLUSAO[w.situacao],
      })))
    }
  }
  if (lidos.length && falhas === lidos.length) throw new Error('gh run list falhou em todos os commits')
  const visiveis = runsVisiveis(porCommit)
  if (aguardando && visiveis.some(r => r.headSha === aguardando?.sha)) aguardando = null
  const workflows = await Promise.all(visiveis.map(r => workflowDe($, r, antes?.workflows.find(w => w.id === r.databaseId))))
  const mesma = antes?.branch === branch ? antes : null
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
    void podar($).catch(err => $.ui.log(`github-actions: poda: ${String(err)}`, { to: 'debug' }))
    void atualizar($)
    return r
  })

  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    const r = await next(e)
    if (!disparaRun(e.command)) return r
    // Push que falhou não registra: o commit não chegou ao GitHub.
    if (ehPush(e.command) && 'result' in r && !r.isError) {
      try {
        const [sha, branch] = await Promise.all([
          texto($, ['git', 'rev-parse', 'HEAD']), texto($, ['git', 'branch', '--show-current'])])
        if (sha && branch) {
          aguardando = { sha, ate: (await $.clock.now()) + PRAZO_PUSH_MS }
          await guardarCommit($, { sha, branch })
        }
      } catch (err) {
        $.ui.log(`github-actions: não guardei o commit empurrado: ${String(err)}`, { to: 'debug' })
      }
    }
    agendar($, ESPERA_PUSH_MS)
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
