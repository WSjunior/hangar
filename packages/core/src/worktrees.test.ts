import { describe, expect, it } from 'vitest';
import { worktreeLabel, worktreesSizeTotal } from './api';

describe('worktreeLabel', () => {
  it('usa a pasta da worktree onde o agente está', () => {
    expect(worktreeLabel({ name: 's', worktree: true, worktree_path: '/r/hangar-rust-parte3', cwd: '/r/hangar' } as never))
      .toBe('hangar-rust-parte3');
  });
  it('nada fora de worktree', () => {
    expect(worktreeLabel({ name: 's', cwd: '/r/hangar' } as never)).toBeNull();
  });
});

describe('worktreesSizeTotal', () => {
  const w = (o: object) => ({ size: null, size_pending: false, size_error: false, size_partial: false, ...o }) as never;
  it('soma o medido e é exato quando tudo foi medido', () => {
    expect(worktreesSizeTotal([w({ size: 10 }), w({ size: 5 })])).toEqual({ bytes: 15, partial: false });
  });
  it('pendente, falha ou parcial fazem do total um mínimo', () => {
    for (const o of [{ size_pending: true }, { size_error: true }, { size: 3, size_partial: true }]) {
      expect(worktreesSizeTotal([w({ size: 10 }), w(o)]).partial).toBe(true);
    }
  });
  it('nada medido soma zero', () => {
    expect(worktreesSizeTotal([w({ size_pending: true })]).bytes).toBe(0);
  });
});
