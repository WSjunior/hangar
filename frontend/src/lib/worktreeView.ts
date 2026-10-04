import { fmtBytes, type State, type WorktreeState, type WorktreeStatus } from '@hangar/core';
import * as m from '../paraglide/messages';

// Tamanho de uma worktree: medição falha ou parcial aparece como tal, nunca como número cheio.
export function worktreeSizeLabel(w: WorktreeStatus): string {
  if (!w.exists) return '—';
  if (w.size_pending) return m.worktrees_disco_calculando();
  if (w.size_error) return m.worktree_tamanho_falhou();
  if (w.size == null) return '—';
  return w.size_partial ? m.worktree_tamanho_parcial({ tamanho: fmtBytes(w.size) }) : fmtBytes(w.size);
}

export const worktreesSizeBytes = (ws: WorktreeStatus[]) => ws.reduce((a, w) => a + (w.size ?? 0), 0);

// Soma de várias: se alguma não foi medida inteira, o total é um mínimo ("≥").
export function worktreesSizeSum(ws: WorktreeStatus[]): string {
  const total = fmtBytes(worktreesSizeBytes(ws));
  return ws.some((w) => w.size_partial || w.size_error || w.size_pending) ? m.worktree_tamanho_parcial({ tamanho: total }) : total;
}

// Rótulo e cor de cada estado da worktree, iguais na lista, no disco e na janela de detalhe.
export const worktreeStateLabel: Record<WorktreeState, () => string> = {
  gone: m.worktree_estado_sumida,
  session: m.worktree_estado_em_uso,
  dirty: m.worktree_estado_nao_commitado,
  merged: m.worktree_estado_mesclada,
  detached: m.worktree_estado_sem_branch,
  active: m.worktree_estado_andamento,
};

export const worktreeStateColor: Record<WorktreeState, string> = {
  gone: 'var(--error)',
  session: 'var(--accent)',
  dirty: 'var(--warning)',
  merged: 'var(--success)',
  detached: 'var(--border-strong)',
  active: 'var(--text-muted)',
};

export function sessionStateLabel(s: State | undefined): string {
  if (s === 'working') return m.worktree_sessao_trabalhando();
  if (s === 'awaiting_input') return m.worktree_sessao_esperando();
  return s ? m.worktree_sessao_parada() : '';
}

// O hash leva o servidor: o App seleciona o dono da sessão antes de montar o Chat.
export function goToSession(serverId: string, name: string) {
  window.location.hash = `#/chat/${encodeURIComponent(serverId)}/${encodeURIComponent(name)}`;
}
