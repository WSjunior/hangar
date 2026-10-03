import type { WorktreeStatus } from '@hangar/core';

// O card não roda git: o ✓ de "juntada" vem do que a tela ou a janela da worktree já leu.
const mapa = $state<Record<string, WorktreeStatus>>({});
const chave = (serverId: string, path: string) => `${serverId}::${path}`;

export const worktreeStatus = {
  get: (serverId: string, path: string) => mapa[chave(serverId, path)],
  put: (serverId: string, st: WorktreeStatus) => { mapa[chave(serverId, st.path)] = st; },
  drop: (serverId: string, path: string) => { delete mapa[chave(serverId, path)]; },
};
