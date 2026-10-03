import { describe, expect, it } from 'vitest';
import { worktreeStatus } from './worktreeStatus.svelte';

describe('worktreeStatus', () => {
  it('guarda e esquece por servidor e caminho', () => {
    const st = { path: '/r/x', repo: '/r', exists: true, branch: 'x', base: 'main', main_branch: 'main', merged: true,
                 ahead: 0, dirty: 0, ignored: [], sessions: [], closed: 0 };
    worktreeStatus.put('srv', st);
    expect(worktreeStatus.get('srv', '/r/x')?.merged).toBe(true);
    expect(worktreeStatus.get('outro', '/r/x')).toBeUndefined();
    worktreeStatus.drop('srv', '/r/x');
    expect(worktreeStatus.get('srv', '/r/x')).toBeUndefined();
  });
});
