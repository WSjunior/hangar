import { describe, expect, it, vi } from 'vitest';
vi.mock('expo-router', () => ({ router: { push: vi.fn() } }));
vi.mock('../../stores/sessions', () => ({ useSessions: () => undefined }));
vi.mock('../../paraglide/messages', () => ({ worktree_subagente_nome: (p: { id: string }) => `Subagente ${p.id}` }));
import { displayTitle } from './worktreeUi';

const w = (path: string) => ({ path, branch: 'worktree-agent-ab1a2b3c4d' }) as never;

describe('displayTitle', () => {
  it('nomeia o subagente pelo id da pasta no Linux e no Windows', () => {
    expect(displayTitle(w('/r/.claude/worktrees/agent-ab1a2b3c4d'))).toBe('Subagente ab1a');
    expect(displayTitle(w('C:\\r\\.claude\\worktrees\\agent-ab1a2b3c4d\\'))).toBe('Subagente ab1a');
  });
});
