// Resumo de cota de UMA conta pra caber numa linha de seletor ("5h 42% · 7d 18%"). Shape mínimo
// do /api/cotas (backend/app/cotas.py, `CotaConta`): o front web tem o tipo completo em
// lib/contaEstado; o app nativo só precisa disto.
export interface JanelaCotaResumo {
  rotulo: string;
  pct: number;
  reset_ts?: number | null;
  por_modelo?: boolean;
}

export interface CotaContaResumo {
  id: string;
  estado: 'lida' | 'sem_credencial' | 'expirada' | 'indisponivel';
  janelas: JanelaCotaResumo[];
  /** Código do backend (`sessao-viva`, `renovacao-falhou`, …), nunca texto de tela. */
  motivo?: string | null;
}

/** Leitura parou porque a credencial não renovou: resolve abrindo uma sessão na conta. */
export function cotaParada(c: CotaContaResumo | undefined): boolean {
  return c?.motivo === 'renovacao-falhou';
}

/** A cota da conta Claude cujo config dir é `path` — a chave do /api/cotas é `claude:<path>`. */
export function cotaDaConta<T extends { id: string }>(cotas: T[], path: string): T | undefined {
  return cotas.find((c) => c.id === `claude:${path}`);
}

/** "5h 42% · 7d 18%"; string vazia sem leitura ou sem janela (quem chama decide o texto). */
export function resumoCota(c: CotaContaResumo | undefined): string {
  if (!c || c.estado !== 'lida') return '';
  return c.janelas
    .filter((j) => typeof j.pct === 'number' && isFinite(j.pct))
    .map((j) => `${j.rotulo} ${Math.round(j.pct)}%`)
    .join(' · ');
}
