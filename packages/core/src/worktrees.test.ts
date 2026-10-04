import { describe, expect, it } from 'vitest';
import { worktreeLabel } from './api';

describe('worktreeLabel', () => {
  it('usa a pasta da worktree onde o agente está', () => {
    expect(worktreeLabel({ name: 's', worktree: true, worktree_path: '/r/hangar-rust-parte3', cwd: '/r/hangar' } as never))
      .toBe('hangar-rust-parte3');
  });
  it('nada fora de worktree', () => {
    expect(worktreeLabel({ name: 's', cwd: '/r/hangar' } as never)).toBeNull();
  });
});
