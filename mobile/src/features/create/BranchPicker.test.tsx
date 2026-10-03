// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
vi.mock('react-native', async (original) => ({ ...await original(),
  Pressable: (p: { accessibilityLabel?: string; onPress?: () => void; children?: unknown }) =>
    createElement('button', { 'aria-label': p.accessibilityLabel, onClick: p.onPress }, p.children as never),
  TextInput: (p: { accessibilityLabel?: string; onChangeText?: (t: string) => void }) =>
    createElement('input', { 'aria-label': p.accessibilityLabel, onChange: (e: { target: { value: string } }) => p.onChangeText?.(e.target.value) }) }));
vi.mock('../../paraglide/messages', () => ({
  native_create_checkout_current: () => 'atual', native_create_checkout_worktree: () => 'worktree',
  worktree_nova_branch: ({ base }: { base: string }) => `nova:${base}`, worktree_nome_branch: () => 'nome',
  worktree_base: () => 'base', worktree_modo: () => 'modo', worktree_modo_ajuda: () => 'ajuda' }));
vi.mock('@hangar/core', async (original) => ({ ...await original(),
  getFolderBranchesForServer: () => Promise.resolve({ current: 'main', branches: ['main', 'x'], remotes: [], dirty: false }) }));
import { BranchPicker } from './BranchPicker';

describe('BranchPicker', () => {
  it('escolher branch nova devolve new_branch com a base atual e o nome da sessão', async () => {
    const onChange = vi.fn();
    const el = document.createElement('div');
    await act(async () => { createRoot(el).render(createElement(BranchPicker,
      { server: { id: 's' } as never, cwd: '/r', sessionName: 'rust-parte3', value: null, onChange })); });
    await act(async () => { (el.querySelector('[aria-label="nova:main"]') as HTMLElement).click(); });
    expect(onChange).toHaveBeenLastCalledWith({ branch: 'rust-parte3', new_branch: true, base: 'main' });
  });
});
