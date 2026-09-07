// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import Composer from './Composer.svelte';
import * as api from '../lib/api';
import * as m from '../paraglide/messages';

vi.mock('../lib/aquecimento', () => ({ aoAquecer: () => Promise.resolve() }));
vi.mock('../lib/api', async (original) => ({
  ...await original<typeof api>(),
  getCommands: vi.fn().mockResolvedValue([{ name: 'revisar', display: '/revisar', source: 'skill', description: 'Revisão' }]),
  getCodexModels: vi.fn().mockResolvedValue({ models: [], current: { model: 'gpt-6-astra', effort: 'high', mode: 'default' } }),
  setCodexMode: vi.fn().mockImplementation(async (_s, mode) => ({ model: 'gpt-6-astra', effort: 'high', mode })),
}));
let componentes: ReturnType<typeof mount>[];
async function flush() { await tick(); await new Promise(r => setTimeout(r, 0)); await tick(); }
const button = (label: string) => [...document.querySelectorAll('button')].find(b => b.textContent?.trim() === label)!;
async function montar() {
  const props = $state({
    sessionName: 'codex-test', sessionState: 'working' as const, provider: 'codex' as const,
    status: { raw: '', model: 'gpt-6-astra', effort: 'high' }, codexMode: 'default' as 'default' | 'plan',
    inputText: '', onSend: vi.fn().mockResolvedValue(undefined), onSteer: vi.fn(), filaCount: 1,
    onCommand: vi.fn(), onInterrupt: vi.fn(), onOpenGit: vi.fn(), onOpenPreview: vi.fn(),
  });
  const el = document.createElement('div'); document.body.appendChild(el);
  componentes.push(mount(Composer, { target: el, props }));
  await flush();
  return props;
}
beforeEach(() => {
  componentes = []; vi.clearAllMocks();
  vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status: 200 }));
});
afterEach(async () => { for (const c of componentes) await unmount(c); document.body.innerHTML = ''; });

it('Shift+Tab alterna Planejar e Normal sem mudar permissões', async () => {
  await montar();
  const textarea = document.querySelector('textarea')!;
  textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true }));
  await flush();
  expect(api.setCodexMode).toHaveBeenLastCalledWith('codex-test', 'plan');
  expect(button(m.codex_modo_plan()).getAttribute('aria-pressed')).toBe('true');
  button(m.codex_modo_plan()).click(); await flush();
  button(m.codex_modo_normal()).click(); await flush();
  expect(api.setCodexMode).toHaveBeenLastCalledWith('codex-test', 'default');
});

it('mudanças recebidas do terminal atualizam o esforço e o modo', async () => {
  const props = await montar();
  expect(button('high')).toBeTruthy();
  props.status = { raw: '', model: 'gpt-5.6-sol', effort: 'xhigh' }; props.codexMode = 'plan';
  await flush();
  expect(button('xhigh')).toBeTruthy();
  expect([...document.querySelectorAll('.pill-model')].some(e => e.textContent === 'gpt-5.6-sol')).toBe(true);
  expect(button(m.codex_modo_plan())).toBeTruthy();
});

it('lista skills com barra e preenche argumentos antes do envio', async () => {
  const props = await montar();
  props.inputText = '/rev'; await flush();
  const skill = [...document.querySelectorAll('button')].find(b => b.textContent?.includes('/revisar'))!;
  expect(skill).toBeTruthy(); skill.click(); await flush();
  expect(document.querySelector('textarea')!.value).toBe('/revisar ');
  expect(props.onSend).not.toHaveBeenCalled();
});

it('permite orientar agora ou enviar à fila e conserva o texto em caso de falha', async () => {
  const props = await montar();
  props.inputText = 'corrigir'; await flush();
  const orient = [...document.querySelectorAll('button')].find(b => b.title === m.codex_orientar_ajuda())!;
  orient.click(); await flush();
  expect(props.onSend).toHaveBeenLastCalledWith('corrigir', true);
  props.inputText = 'depois'; await flush();
  document.querySelector<HTMLButtonElement>('button.send-btn')!.click(); await flush();
  expect(props.onSend).toHaveBeenLastCalledWith('depois', false);
  props.onSend.mockRejectedValueOnce(new Error('Turno encerrado'));
  props.inputText = 'preservar'; await flush();
  [...document.querySelectorAll('button')].find(b => b.title === m.codex_orientar_ajuda())!.click(); await flush();
  expect(document.querySelector('textarea')!.value).toBe('preservar');
});
