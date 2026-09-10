// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { mount, unmount, tick, createRawSnippet } from 'svelte';
import Archive from './Archive.svelte';
import * as api from '@hangar/core';
import * as m from '../paraglide/messages';

vi.mock('../components/MessageList.svelte', () => ({ default: createRawSnippet(() => ({ render: () => '<div>history</div>' })) }));
vi.mock('../components/NavBar.svelte', () => ({ default: createRawSnippet(() => ({ render: () => '<nav></nav>' })) }));
vi.mock('../lib/queries', () => ({ arquivo: vi.fn(), clienteQuery: { fetchQuery: vi.fn(async () => []) } }));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  getArchiveFolder: vi.fn(), getArchiveHistory: vi.fn(async () => []),
  getEngines: vi.fn(async () => ({ motores: {} })),
  resumeArchivedConversation: vi.fn(async () => ({ name: 'resumed' })),
}));

it('deep-link usa conta/provider dos metadados na leitura e retomada', async () => {
  const b = { id: 'B', label: 'B', baseUrl: 'https://b.test', token: 'b' };
  localStorage.setItem('cp_servers', JSON.stringify([b])); localStorage.setItem('cp_active', 'B');
  vi.mocked(api.getArchiveFolder).mockResolvedValue([{
    project: 'project', session_id: 'thread', cwd: '/test', mtime: 1, preview: '', ultima: 'test', live: false,
    config_dir: null, conta: 'Work', provider: 'codex', codex_account: 'work', codex_home: '/test/work',
  }]);
  const target = document.body.appendChild(document.createElement('div'));
  const component = mount(Archive, { target, props: { onBack: vi.fn(), deepLink: { serverId: 'B', project: 'project', sessionId: 'thread' } } });
  try {
    for (let i = 0; i < 15; i++) await tick();
    expect(api.getArchiveHistory).toHaveBeenCalledWith('project', 'thread', undefined, null, 'codex', 'work', b);
    expect(target.textContent).toContain('history');
    target.querySelector<HTMLButtonElement>('.resume-btn')!.click();
    for (let i = 0; i < 10; i++) await tick();
    expect(api.resumeArchivedConversation).toHaveBeenCalledWith('project', 'thread', null, null, 'codex', 'work', b);
    expect(location.hash).toBe('#/chat/B/resumed');
  } finally { await unmount(component); target.remove(); localStorage.clear(); location.hash = ''; }
});

it('deep-link ambíguo não lê nem retoma a primeira conta', async () => {
  vi.clearAllMocks();
  const b = { id: 'B', label: 'B', baseUrl: 'https://b.test', token: 'b' };
  localStorage.setItem('cp_servers', JSON.stringify([b])); localStorage.setItem('cp_active', 'B');
  vi.mocked(api.getArchiveFolder).mockResolvedValue(['personal', 'work'].map((account) => ({
    project: 'project', session_id: 'same-thread', cwd: '/test', mtime: 1, preview: '', ultima: 'test', live: false,
    config_dir: null, conta: account, provider: 'codex', codex_account: account, codex_home: `/test/${account}`,
  })));
  const target = document.body.appendChild(document.createElement('div'));
  const component = mount(Archive, { target, props: { onBack: vi.fn(), deepLink: { serverId: 'B', project: 'project', sessionId: 'same-thread' } } });
  try {
    for (let i = 0; i < 15; i++) await tick();
    expect(api.getArchiveHistory).not.toHaveBeenCalled();
    expect(api.resumeArchivedConversation).not.toHaveBeenCalled();
    expect(target.textContent).toContain(m.codex_account_ambiguous_rollout());
  } finally { await unmount(component); target.remove(); localStorage.clear(); location.hash = ''; }
});
