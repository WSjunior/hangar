import { describe, expect, it } from 'vitest';
import type { WorktreeRepo } from '@hangar/core';
import { reposAfterError } from './worktreeView';

describe('reposAfterError', () => {
  it('releitura que falha mantém a lista que já estava na tela', () => {
    const repos = [{ repo: '/r/a', worktrees: [] }] as unknown as WorktreeRepo[];
    const blocos = [{ servidor: { id: 's1' }, repos }, { servidor: { id: 's2' }, repos: [] }];
    expect(reposAfterError(blocos, 's1')).toBe(repos);
  });
  it('primeira leitura que falha fica vazia', () => {
    expect(reposAfterError([], 's1')).toEqual([]);
  });
});
