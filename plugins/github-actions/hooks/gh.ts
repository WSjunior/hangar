// Tradução do JSON do `gh` para o que a faixa desenha. Sem `$`: tudo aqui é puro e testável.

import type { Checks, GhView, Job, Pr, Situacao } from '../types'

export type { GhView, Job, Pr, Situacao, Workflow } from '../types'

export type RunGh = {
  databaseId: number
  workflowName: string
  status: string
  conclusion: string
  headSha: string
  url: string
}
type StepGh = { name: string; status: string; conclusion: string }
export type JobGh = { name: string; status: string; conclusion: string; steps?: StepGh[] }

/** status/conclusion do Actions (minúsculo no REST, maiúsculo no GraphQL do PR). */
export function situacao(status: string, conclusion: string | null | undefined): Situacao {
  const s = status.toLowerCase()
  if (s === 'in_progress') return 'rodando'
  if (s !== 'completed') return 'esperando'
  const c = (conclusion ?? '').toLowerCase()
  if (c === 'success' || c === 'neutral') return 'ok'
  if (c === 'skipped') return 'pulado'
  if (c === 'cancelled') return 'cancelado'
  return 'falhou'
}

export const emAndamento = (s: Situacao) => s === 'rodando' || s === 'esperando'

/** O passo que interessa: o que está rodando, ou o que falhou. */
function passoDe(j: JobGh, s: Situacao): string | null {
  const steps = j.steps ?? []
  if (s === 'rodando') return steps.find(p => p.status === 'in_progress')?.name ?? null
  if (s === 'falhou') return steps.find(p => situacao(p.status, p.conclusion) === 'falhou')?.name ?? null
  return null
}

export function jobs(lista: readonly JobGh[]): Job[] {
  return lista.map(j => {
    const s = situacao(j.status, j.conclusion)
    return { nome: j.name, situacao: s, passo: passoDe(j, s) }
  })
}

/** Último run de cada workflow, só os do commit mais novo da branch (o `gh` lista do mais novo). */
export function runsAtuais(runs: readonly RunGh[]): RunGh[] {
  const sha = runs[0]?.headSha
  const vistos = new Set<string>()
  return runs.filter(r => {
    if (r.headSha !== sha || vistos.has(r.workflowName)) return false
    vistos.add(r.workflowName)
    return true
  })
}

type CheckGh = { __typename?: string; status?: string; conclusion?: string | null; state?: string }

export function checks(rollup: readonly CheckGh[] | null | undefined): Checks {
  const r: Checks = { ok: 0, falhou: 0, rodando: 0 }
  for (const c of rollup ?? []) {
    // StatusContext (status externo) só tem `state`; CheckRun tem status + conclusion.
    const s = c.__typename === 'StatusContext'
      ? ({ SUCCESS: 'ok', PENDING: 'esperando', EXPECTED: 'esperando' } as Record<string, Situacao>)[c.state ?? ''] ?? 'falhou'
      : situacao(c.status ?? '', c.conclusion)
    if (s === 'ok' || s === 'pulado') r.ok++
    else if (emAndamento(s)) r.rodando++
    else r.falhou++
  }
  return r
}

export type PrGh = {
  number: number
  title: string
  url: string
  state: Pr['estado']
  isDraft: boolean
  reviewDecision: string
  statusCheckRollup?: CheckGh[]
}

export function pr(p: PrGh): Pr {
  const rev = p.reviewDecision
  return {
    numero: p.number,
    titulo: p.title,
    url: p.url,
    estado: p.state,
    rascunho: p.isDraft,
    revisao: rev === 'APPROVED' || rev === 'CHANGES_REQUESTED' || rev === 'REVIEW_REQUIRED' ? rev : null,
    checks: checks(p.statusCheckRollup),
  }
}

/** Ainda há o que acompanhar: run ou check do PR que não terminou. */
export function precisaConsultar(v: GhView | null): boolean {
  if (!v) return false
  return v.workflows.some(w => emAndamento(w.situacao) || w.jobs.some(j => emAndamento(j.situacao)))
    || (v.pr?.estado === 'OPEN' && v.pr.checks.rodando > 0)
}

/** Comando Bash que costuma criar run ou PR novo: vale consultar logo depois. */
export const disparaRun = (cmd: string) => /\bgit\s+push\b|\bgh\s+(pr|run|workflow)\b/.test(cmd)

/** Endereço de repositório no GitHub (https ou ssh). */
export const ehGithub = (remoto: string) => /(^|[@/])github\.com[:/]/.test(remoto.trim())
