// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Login from './CodexContaLogin.svelte';
import Harness from './CodexContaLogin.harness.svelte';
import * as api from '@hangar/core';
import * as m from '../../paraglide/messages';

vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  createCodexAccountForServer: vi.fn(), prepareCodexAccountForServer: vi.fn(),
  getCodexPreparationForServer: vi.fn(), startCodexAccountLoginForServer: vi.fn(),
  getCodexAccountLoginForServer: vi.fn(), cancelCodexAccountLoginForServer: vi.fn(),
}));
const A = { id: 'A', label: 'A', baseUrl: 'https://a.test', token: 'a' };
const B = { id: 'B', label: 'B', baseUrl: 'https://b.test', token: 'b' };
const waiting = { account_id: 'default', attempt_id: 'attempt-A', status: 'waiting' as const, user_code: 'ABC', verification_url: 'https://auth.openai.com/device' };
const ready = { status: 'ready' as const, trust_pending: false, issues: [] };
let components: ReturnType<typeof mount>[] = [];
const flush = async () => { for (let i = 0; i < 8; i++) await tick(); };
function button(text: string) { return [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)!; }
function setup(accountId: string | undefined = 'default') {
  const target = document.body.appendChild(document.createElement('div'));
  const done = vi.fn();
  const component = mount(Login, { target, props: { server: B, accountId, oncomplete: done } });
  components.push(component);
  return { done, component };
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(api.getCodexAccountLoginForServer).mockResolvedValue(null);
  vi.mocked(api.prepareCodexAccountForServer).mockResolvedValue(ready);
  vi.mocked(api.startCodexAccountLoginForServer).mockResolvedValue(waiting);
});
afterEach(async () => { for (const c of components) await unmount(c); components = []; document.body.innerHTML = ''; });

it('prepara e inicia no servidor B, sem criar a padrão; fechar não cancela nem confirma', async () => {
  const { component, done } = setup(); await flush();
  button(m.contas_entrar()).click(); await flush();
  expect(api.prepareCodexAccountForServer).toHaveBeenCalledWith(B, 'default');
  expect(api.startCodexAccountLoginForServer).toHaveBeenCalledWith(B, 'default');
  expect(api.createCodexAccountForServer).not.toHaveBeenCalled();
  expect(document.body.textContent).toContain('ABC');
  await unmount(component); components = [];
  expect(api.cancelCodexAccountLoginForServer).not.toHaveBeenCalled();
  expect(done).not.toHaveBeenCalled();
});

it('cancelar A captura ID e não altera a tentativa B após trocar servidor', async () => {
  vi.mocked(api.getCodexAccountLoginForServer).mockImplementation(async (s) => ({ ...waiting, attempt_id: s.id }));
  let resolve!: (a: api.CodexLoginAttempt) => void;
  vi.mocked(api.cancelCodexAccountLoginForServer).mockReturnValue(new Promise((r) => resolve = r));
  const target = document.body.appendChild(document.createElement('div'));
  components.push(mount(Harness, { target, props: { servers: [A, B], oncomplete: vi.fn() } }));
  await flush(); button(m.codex_ui_cancel_login()).click(); await flush();
  button('switch').click(); await flush();
  resolve({ ...waiting, attempt_id: 'A', status: 'cancelled' }); await flush();
  expect(api.cancelCodexAccountLoginForServer).toHaveBeenCalledExactlyOnceWith(A, 'default', 'A');
  expect(document.body.textContent).not.toContain(m.codex_ui_cancelled());
  expect(document.body.textContent).toContain(m.novacred_codex_aguardando());
});

it('resposta atrasada de A não mostra código nem conclusão em B', async () => {
  let resolve!: (a: api.CodexLoginAttempt) => void;
  vi.mocked(api.getCodexAccountLoginForServer).mockImplementation((s) => s.id === 'A'
    ? new Promise((r) => resolve = r) : Promise.resolve(null));
  const done = vi.fn(), target = document.body.appendChild(document.createElement('div'));
  components.push(mount(Harness, { target, props: { servers: [A, B], oncomplete: done } }));
  await flush(); button('switch').click(); await flush();
  resolve({ ...waiting, status: 'completed' }); await flush();
  expect(done).not.toHaveBeenCalled(); expect(document.body.textContent).not.toContain('ABC');
});

it('falha de preparo permanece visível e não inicia login', async () => {
  vi.mocked(api.prepareCodexAccountForServer).mockResolvedValue({ ...ready, status: 'error' });
  setup(); await flush(); button(m.contas_entrar()).click(); await flush();
  expect(document.body.textContent).toContain(m.codex_ui_prepare_error());
  expect(api.startCodexAccountLoginForServer).not.toHaveBeenCalled();
});

it('cria conta nomeada e só então prepara e solicita login', async () => {
  vi.mocked(api.createCodexAccountForServer).mockResolvedValue({ id: 'work' } as api.CodexAccount);
  const target = document.body.appendChild(document.createElement('div'));
  components.push(mount(Login, { target, props: { server: B, oncomplete: vi.fn() } })); await flush();
  const input = document.querySelector('input')!; input.value = 'work'; input.dispatchEvent(new Event('input')); await flush();
  button(m.contas_entrar()).click(); await flush();
  expect(api.createCodexAccountForServer).toHaveBeenCalledWith(B, 'work');
  expect(api.prepareCodexAccountForServer).toHaveBeenCalledWith(B, 'work');
  expect(api.startCodexAccountLoginForServer).toHaveBeenCalledWith(B, 'work');
});

it('recusa link não HTTPS e expõe erro de consulta', async () => {
  vi.mocked(api.getCodexAccountLoginForServer).mockResolvedValue({ ...waiting, verification_url: 'javascript:alert(1)' });
  setup(); await flush(); expect(document.querySelector('a')).toBeNull();
  await unmount(components.pop()!);
  vi.mocked(api.getCodexAccountLoginForServer).mockRejectedValue(new Error(m.codex_ui_login_error()));
  setup(); await flush(); expect(document.body.textContent).toContain(m.codex_ui_login_error());
});
