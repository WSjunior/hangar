export type Falha = 'login' | 'limite' | 'outra'
export type Situacao = 'ok' | 'falhou' | 'rodando' | 'esperando' | 'pulado' | 'cancelado'
// `feitos`/`total`: passos do job terminados e existentes; `inicio`/`fim`: do run, em ms.
export type Job = { nome: string; situacao: Situacao; passo: string | null; feitos: number; total: number }
export type Workflow = {
  id: number; nome: string; sha: string; situacao: Situacao; url: string; jobs: Job[]; inicio: number | null; fim: number | null
}
export type Checks = { ok: number; falhou: number; rodando: number }
export type Pr = {
  numero: number
  titulo: string
  url: string
  estado: 'OPEN' | 'MERGED' | 'CLOSED'
  rascunho: boolean
  revisao: 'APPROVED' | 'CHANGES_REQUESTED' | 'REVIEW_REQUIRED' | null
  checks: Checks
}
// `aviso`/`falha`: por que a última consulta falhou; a faixa o mostra e segue tentando.
export type GhView = { branch: string; workflows: Workflow[]; pr: Pr | null; aviso?: string | null; falha?: Falha | null }

declare module 'claude-code' {
  interface PluginState {
    'github-actions': { view: GhView | null; recolhida: boolean }
  }
}
