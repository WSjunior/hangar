// @vitest-environment happy-dom
import { expect, it, vi } from 'vitest';
import { mount, unmount, tick, createRawSnippet } from 'svelte';
import Archive from './Archive.svelte';
import Harness from './Archive.harness.svelte';
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
    expect(target.textContent).toContain(m.arquivo_conversa_conta_ambigua());
  } finally { await unmount(component); target.remove(); localStorage.clear(); location.hash = ''; }
});

it('trocar o deep-link libera o carregamento antigo', async () => {
  vi.clearAllMocks();
  const servers = [
    { id: 'B', label: 'B', baseUrl: 'https://b.test', token: 'b' },
    { id: 'C', label: 'C', baseUrl: 'https://c.test', token: 'c' },
  ];
  localStorage.setItem('cp_servers', JSON.stringify(servers)); localStorage.setItem('cp_active', 'B');
  let resolveHistory!: (events: api.ChatEvent[]) => void;
  vi.mocked(api.getArchiveFolder).mockImplementation(async (project) => project === 'project-b' ? [{
    project, session_id: 'thread-b', cwd: '/b', mtime: 1, preview: '', ultima: 'B', live: false,
    config_dir: null, conta: 'B', provider: 'codex', codex_account: 'b', codex_home: '/b',
  }] : []);
  vi.mocked(api.getArchiveHistory).mockReturnValueOnce(new Promise((resolve) => { resolveHistory = resolve; }));
  const target = document.body.appendChild(document.createElement('div'));
  const component = mount(Harness, { target, props: { links: [
    { serverId: 'B', project: 'project-b', sessionId: 'thread-b' },
    { serverId: 'C', project: 'project-c', sessionId: 'thread-c' },
  ] } });
  try {
    for (let i = 0; i < 15; i++) await tick();
    expect(target.textContent).toContain(m.arquivo_carregando());
    target.querySelector<HTMLButtonElement>('button')!.click();
    for (let i = 0; i < 15; i++) await tick();
    resolveHistory([]);
    for (let i = 0; i < 10; i++) await tick();
    expect(target.textContent).not.toContain(m.arquivo_carregando());
  } finally { await unmount(component); target.remove(); localStorage.clear(); }
});
