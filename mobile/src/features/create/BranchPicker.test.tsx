// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock('react-native', async (original) => ({ ...await original<typeof import('react-native')>(),
  Pressable: (p: { accessibilityLabel?: string; onPress?: () => void; children?: unknown }) =>
    createElement('button', { 'aria-label': p.accessibilityLabel, onClick: p.onPress }, p.children as never),
  TextInput: (p: { accessibilityLabel?: string; onChangeText?: (t: string) => void }) =>
    createElement('input', { 'aria-label': p.accessibilityLabel, onChange: (e: { target: { value: string } }) => p.onChangeText?.(e.target.value) }) }));
vi.mock('../../paraglide/messages', () => ({
  native_create_checkout_current: () => 'atual', native_create_checkout_worktree: () => 'worktree',
  worktree_nova_branch: ({ base }: { base: string }) => `nova:${base}`, worktree_nome_branch: () => 'nome',
  worktree_base: () => 'base', worktree_modo: () => 'modo', worktree_modo_ajuda: () => 'ajuda',
  native_create_checkout_loading: () => 'lendo', native_create_checkout_failed: ({ reason }: { reason: string }) => `falha:${reason}` }));
const folder = vi.hoisted(() => ({ value: { current: 'main' as string | null, branches: ['main', 'x'], remotes: [] as string[], dirty: false } }));
vi.mock('@hangar/core', async (original) => ({ ...await original<typeof import('@hangar/core')>(),
  getFolderBranchesForServer: () => Promise.resolve(folder.value) }));
import { BranchPicker } from './BranchPicker';

describe('BranchPicker', () => {
  // Sem nome digitado a branch vai vazia: o store a preenche com o nome final da sessão.
  it('escolher branch nova devolve new_branch com a base atual e sem nome digitado', async () => {
    const onChange = vi.fn();
    const el = document.createElement('div');
    await act(async () => { createRoot(el).render(createElement(BranchPicker,
      { server: { id: 's' } as never, cwd: '/r', sessionName: 'rust-parte3', value: null, onChange })); });
    await act(async () => { (el.querySelector('[aria-label="nova:main"]') as HTMLElement).click(); });
    expect(onChange).toHaveBeenLastCalledWith({ branch: '', new_branch: true, base: 'main' });
  });

  it('remontar com uma branch nova escolhida mantém o nome digitado e a base', async () => {
    const onChange = vi.fn();
    const el = document.createElement('div');
    await act(async () => { createRoot(el).render(createElement(BranchPicker,
      { server: { id: 's' } as never, cwd: '/r', sessionName: 'rust-parte3',
        value: { branch: 'x', new_branch: true, base: 'main' }, onChange })); });
    expect(el.querySelector('[aria-label="nome"]')).toBeTruthy();
    expect(onChange).toHaveBeenLastCalledWith({ branch: 'x', new_branch: true, base: 'main' });
    expect(onChange).not.toHaveBeenCalledWith(null);
  });

  it('HEAD solto: a base da branch nova é a primeira branch de verdade', async () => {
    folder.value = { current: null, branches: ['dev', 'main'], remotes: [], dirty: false };
    try {
      const onChange = vi.fn();
      const el = document.createElement('div');
      await act(async () => { createRoot(el).render(createElement(BranchPicker,
        { server: { id: 's' } as never, cwd: '/r', sessionName: 's', value: null, onChange })); });
      await act(async () => { (el.querySelector('[aria-label="nova:dev"]') as HTMLElement).click(); });
      expect(onChange).toHaveBeenLastCalledWith({ branch: '', new_branch: true, base: 'dev' });
    } finally {
      folder.value = { current: 'main', branches: ['main', 'x'], remotes: [], dirty: false };
    }
  });
});
