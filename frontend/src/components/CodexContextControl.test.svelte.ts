// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import Control from './CodexContextControl.svelte';

const server = { id: 'a', label: 'A', baseUrl: 'http://a.local', token: 'teste' };
const response = (enabled: boolean) => new Response(JSON.stringify({ contexto_estendido: enabled }));
let component: ReturnType<typeof mount>;
async function flush() { for (let i = 0; i < 20; i++) await tick(); }
const toggle = () => document.querySelector<HTMLInputElement>('[role="switch"]')!;
async function open() {
  const props = $state({ server, busy: false });
  component = mount(Control, { target: document.body, props });
  await flush();
  return props;
}
afterEach(async () => { await unmount(component); vi.restoreAllMocks(); document.body.innerHTML = ''; });

it('lê o padrão, salva ao tocar e só libera criação depois da confirmação', async () => {
  let finish: (r: Response) => void = () => {};
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => init?.method === 'POST'
    ? new Promise<Response>(resolve => { finish = resolve; }) : response(true));
  const props = await open();
  expect(toggle().checked).toBe(true);
  toggle().click(); await flush();
  expect(props.busy).toBe(true);
  expect(toggle().disabled).toBe(true);
  expect(toggle().checked).toBe(true);
  expect(fetch).toHaveBeenLastCalledWith('http://a.local/api/harness/codex/opcoes', expect.objectContaining({
    method: 'POST', body: JSON.stringify({ contexto_estendido: false }),
  }));
  finish(response(false)); await flush();
  expect(props.busy).toBe(false);
  expect(toggle().checked).toBe(false);
});

it('falha de gravação mantém o valor confirmado e mostra o erro', async () => {
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => init?.method === 'POST'
    ? new Response(JSON.stringify({ detail: 'Gravação recusada' }), { status: 409 }) : response(false));
  const props = await open();
  toggle().click(); await flush();
  expect(toggle().checked).toBe(false);
  expect(props.busy).toBe(false);
  expect(document.querySelector('[role="alert"]')?.textContent).toContain('Gravação recusada');
});

it('resposta atrasada não troca o padrão de outro servidor', async () => {
  let finish: (r: Response) => void = () => {};
  vi.spyOn(globalThis, 'fetch').mockImplementation(async url => String(url).includes('a.local')
    ? new Promise<Response>(resolve => { finish = resolve; }) : response(false));
  const props = await open();
  props.server = { ...server, id: 'b', baseUrl: 'http://b.local' }; await flush();
  finish(response(true)); await flush();
  expect(toggle().checked).toBe(false);
  expect(props.busy).toBe(false);
});
