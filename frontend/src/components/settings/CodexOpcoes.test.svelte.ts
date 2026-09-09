// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import CodexOpcoes from './CodexOpcoes.svelte';
import * as m from '../../paraglide/messages';

let components: ReturnType<typeof mount>[];
const server = { id: 'b', label: 'B', baseUrl: 'http://b.local', token: 'token-teste' };
const data = (enabled: boolean) => ({ contexto_estendido: enabled, contexto_configurado: enabled ? 1000000 : null,
  compactacao: null, modelos: [{ model: 'gpt-6-astra', default: 272000, max: 872000 }] });
async function flush() { for (let i = 0; i < 15; i++) await tick(); }
async function montar() {
  const props = $state({ apiTarget: server, nome: 'Codex', onClose: vi.fn() });
  const el = document.createElement('div'); document.body.appendChild(el);
  components.push(mount(CodexOpcoes, { target: el, props })); await flush(); return props;
}
beforeEach(() => { components = []; });
afterEach(async () => { for (const c of components) await unmount(c); vi.restoreAllMocks(); document.body.innerHTML = ''; });

it('salva no servidor escolhido e recupera a opção ao reabrir', async () => {
  let enabled = false;
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => {
    if (init?.method === 'POST') enabled = JSON.parse(String(init.body)).contexto_estendido;
    return new Response(JSON.stringify(data(enabled)));
  });
  await montar();
  document.querySelector<HTMLInputElement>('[role="switch"]')!.click(); await flush();
  expect(enabled).toBe(false);
  [...document.querySelectorAll('button')].find(b => b.textContent?.trim() === m.ctx_salvar())!.click(); await flush();
  expect(enabled).toBe(true);
  expect(fetch).toHaveBeenCalledWith('http://b.local/api/harness/codex/opcoes', expect.objectContaining({
    method: 'POST', body: JSON.stringify({ contexto_estendido: true }),
  }));
  await unmount(components.pop()!); await montar();
  expect(document.querySelector<HTMLInputElement>('[role="switch"]')!.checked).toBe(true);
});

it('resposta antiga não atravessa a troca de servidor', async () => {
  let respond: (r: Response) => void = () => {};
  vi.spyOn(globalThis, 'fetch').mockImplementation(async url => String(url).includes('b.local')
    ? new Promise<Response>(r => { respond = r; }) : new Response(JSON.stringify(data(false))));
  const props = await montar();
  props.apiTarget = { ...server, id: 'c', baseUrl: 'http://c.local' }; await flush();
  respond(new Response(JSON.stringify(data(true)))); await flush();
  expect(document.querySelector<HTMLInputElement>('[role="switch"]')!.checked).toBe(false);
});
