// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const api = vi.hoisted(() => ({ getWorktreeForServer: vi.fn(), deleteWorktreeForServer: vi.fn() }));
vi.mock('react-native', async (original) => ({ ...await original<typeof import('react-native')>(),
  Pressable: (p: { accessibilityLabel?: string; onPress?: () => void; children?: unknown }) =>
    createElement('button', { 'aria-label': p.accessibilityLabel, onClick: p.onPress }, p.children as never),
  Switch: (p: { accessibilityLabel?: string; value?: boolean; onValueChange?: (v: boolean) => void }) =>
    createElement('input', { type: 'checkbox', 'aria-label': p.accessibilityLabel, checked: !!p.value, onChange: () => p.onValueChange?.(!p.value) }) }));
vi.mock('../../ui/Sheet', () => ({ Sheet: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
vi.mock('../../paraglide/messages', () => ({
  comum_carregando: () => 'carregando', worktrees_erro: ({ motivo }: { motivo: string }) => `erro:${motivo}`,
  worktree_juntada: () => 'juntada', worktree_nao_juntada: ({ n }: { n: number }) => `nao_juntada:${n}`,
  worktree_conversas_fechadas: ({ n }: { n: number }) => `fechadas:${n}`,
  worktree_bloqueada: ({ nomes }: { nomes: string }) => `worktree_bloqueada:${nomes}`,
  worktree_apagar_perde: () => 'perde', worktree_nao_commitados: ({ n }: { n: number }) => `nao_commitados:${n}`,
  worktree_apagar_conversas: () => 'conversas', worktree_apagar_branch_juntada: () => 'branch_juntada',
  worktree_apagar_branch_fica: () => 'branch_fica', worktree_apagar_branch_tambem: () => 'branch_tambem',
  worktree_apagar: () => 'Apagar' }));
vi.mock('@hangar/core', async (original) => ({ ...await original<typeof import('@hangar/core')>(), ...api }));
import { WorktreeSheet } from './WorktreeSheet';

const st = { path: '/r/hangar-x', repo: '/r/hangar', exists: true, branch: 'x', base: 'main', merged: false, ahead: 3,
  dirty: 2, ignored: ['.env.local'], sessions: [] as string[], closed: 0 };

async function montar() {
  const el = document.createElement('div');
  await act(async () => { createRoot(el).render(createElement(WorktreeSheet,
    { server: { id: 's' } as never, path: '/r/hangar-x', onClose: () => {} })); });
  return el;
}

describe('WorktreeSheet', () => {
  it('apagar com arquivos a perder confirma e mantém a branch', async () => {
    api.getWorktreeForServer.mockResolvedValue(st);
    api.deleteWorktreeForServer.mockResolvedValue({ removed: st.path, branch_deleted: false, moved: 0 });
    const el = await montar();
    expect(el.textContent).toContain('nao_commitados:2');
    expect(el.textContent).toContain('.env.local');
    await act(async () => { (el.querySelector('[aria-label="Apagar"]') as HTMLElement).click(); });
    expect(api.deleteWorktreeForServer).toHaveBeenCalledWith(expect.anything(),
      expect.objectContaining({ repo: '/r/hangar', path: '/r/hangar-x', confirm: true, delete_branch: false }));
  });

  it('com sessão aberta não oferece apagar', async () => {
    api.getWorktreeForServer.mockResolvedValue({ ...st, sessions: ['s1'] });
    const el = await montar();
    expect(el.querySelector('[aria-label="Apagar"]')).toBeNull();
    expect(el.textContent).toContain('worktree_bloqueada');
  });
});
