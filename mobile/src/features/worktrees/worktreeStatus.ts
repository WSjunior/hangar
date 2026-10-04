import { useSyncExternalStore } from 'react';
import type { WorktreeStatus } from '@hangar/core';

// A linha não roda git: o ✓ de "mesclada" vem do que a tela ou a folha da worktree já leu.
const mapa = new Map<string, WorktreeStatus>();
const ouvintes = new Set<() => void>();
const chave = (serverId: string, path: string) => `${serverId}::${path}`;
const avisar = () => ouvintes.forEach((f) => f());
const assinar = (f: () => void) => { ouvintes.add(f); return () => { ouvintes.delete(f); }; };

export function putWorktreeStatus(serverId: string, st: WorktreeStatus) { mapa.set(chave(serverId, st.path), st); avisar(); }
export function dropWorktreeStatus(serverId: string, path: string) { mapa.delete(chave(serverId, path)); avisar(); }
export function useWorktreeStatus(serverId: string, path: string | null): WorktreeStatus | undefined {
  return useSyncExternalStore(assinar, () => (path ? mapa.get(chave(serverId, path)) : undefined));
}
