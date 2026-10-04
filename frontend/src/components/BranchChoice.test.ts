// @vitest-environment happy-dom
import { describe, it, expect, vi, afterEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import type { FolderBranches, Server, WorktreeChoice } from '@hangar/core';
import BranchChoice from './BranchChoice.svelte';

const branches = vi.hoisted(() => ({ value: null as FolderBranches | null }));
vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()),
  getFolderBranchesForServer: vi.fn(async () => branches.value),
}));

const server = { id: 's1', label: 's1', baseUrl: 'http://s1', token: 't' } as unknown as Server;
let app: ReturnType<typeof mount> | null = null;

async function flush() {
  for (let i = 0; i < 5; i++) { await Promise.resolve(); await tick(); }
}

async function pickNewBranch(sessionName: string): Promise<WorktreeChoice | null> {
  let last: WorktreeChoice | null = null;
  const target = document.createElement('div');
  document.body.appendChild(target);
  app = mount(BranchChoice, { target, props: { server, cwd: '/r', value: null, sessionName, onChange: (v) => { last = v; } } });
  await flush();
  const select = target.querySelector('select') as HTMLSelectElement;
  // O happy-dom só casa `:checked` em <input>, e o bind:value do Svelte lê a opção escolhida por ele.
  const qs = select.querySelector.bind(select);
  select.querySelector = ((s: string) => (s === ':checked' ? select.options[select.selectedIndex] ?? null : qs(s))) as typeof select.querySelector;
  select.value = 'new';
  select.dispatchEvent(new Event('change'));
  await flush();
  return last;
}

afterEach(() => { if (app) unmount(app); app = null; document.body.innerHTML = ''; });

describe('BranchChoice', () => {
  it('branch nova sem nome digitado usa o nome da sessão limpo', async () => {
    branches.value = { current: 'main', branches: ['main'], remotes: [], dirty: false };
    const v = await pickNewBranch('Minha Sessão');
    expect(v).toEqual({ branch: 'Minha-Sessao', new_branch: true, base: 'main' });
  });

  it('HEAD solto: a base é a primeira branch de verdade', async () => {
    branches.value = { current: null, branches: ['dev', 'main'], remotes: [], dirty: false };
    const v = await pickNewBranch('s');
    expect(v?.base).toBe('dev');
  });
});
