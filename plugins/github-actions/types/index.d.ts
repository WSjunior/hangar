export type Situacao = 'ok' | 'falhou' | 'rodando' | 'esperando' | 'pulado' | 'cancelado'
export type Job = { nome: string; situacao: Situacao; passo: string | null }
export type Workflow = { id: number; nome: string; sha: string; situacao: Situacao; url: string; jobs: Job[] }
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
export type GhView = { branch: string; workflows: Workflow[]; pr: Pr | null }

declare module 'claude-code' {
  interface PluginState {
    'github-actions': { view: GhView | null }
  }
}
