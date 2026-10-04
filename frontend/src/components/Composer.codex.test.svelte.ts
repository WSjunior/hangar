// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import Composer from './Composer.svelte';
import * as api from '@hangar/core';
import * as m from '../paraglide/messages';

vi.mock('../lib/aquecimento', () => ({ aoAquecer: () => Promise.resolve() }));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof api>(),
  getCommands: vi.fn().mockResolvedValue([
    { name: 'revisar', display: '/revisar', source: 'skill', description: 'Revisão' },
    { name: 'resumir', display: '/resumir', source: 'skill', description: 'Resumo' },
  ]),
  getCodexModels: vi.fn().mockResolvedValue({ models: [], current: { model: 'gpt-6-astra', effort: 'high', mode: 'default' } }),
  getCodexPermissions: vi.fn().mockResolvedValue({ modes: [], current: 'Full Access' }),
  setCodexPermission: vi.fn(),
  setCodexMode: vi.fn().mockImplementation(async (_s, mode) => ({ model: 'gpt-6-astra', effort: 'high', mode })),
}));
let componentes: ReturnType<typeof mount>[];
async function flush() { await tick(); await new Promise(r => setTimeout(r, 0)); await tick(); }
const button = (label: string) => [...document.querySelectorAll('button')].find(b => b.textContent?.trim() === label)!;
async function montar() {
  const props = $state({
    sessionName: 'codex-test', sessionState: 'working' as 'working' | 'idle', provider: 'codex' as const,
    status: { raw: '', model: 'gpt-6-astra', effort: 'high' }, codexMode: 'default' as 'default' | 'plan' | null,
    inputText: '', onSend: vi.fn().mockResolvedValue(undefined), onSteer: vi.fn().mockResolvedValue(true), filaCount: 1,
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
  expect(api.setCodexMode).toHaveBeenLastCalledWith('codex-test', 'plan', undefined);
  expect(button(m.codex_modo_plan()).getAttribute('aria-pressed')).toBe('true');
  button(m.codex_modo_plan()).click(); await flush();
  button(m.codex_modo_normal()).click(); await flush();
  expect(api.setCodexMode).toHaveBeenLastCalledWith('codex-test', 'default', undefined);
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

it('Codex headless mostra a permissão conhecida mesmo durante o turno', async () => {
  const props = await montar();
  (props as unknown as { headless: boolean }).headless = true;
  (props as unknown as { estreito: boolean }).estreito = true;
  await flush();
  expect(api.getCodexPermissions).toHaveBeenCalledWith('codex-test', undefined);
  const permission = document.querySelector<HTMLButtonElement>('.pill-duo button[aria-label="Full Access"]');
  expect(permission).not.toBeNull();
  expect(permission?.closest('.status-tab')).toBeNull();
});

it('consulta inicial atrasada não desfaz a permissão confirmada pelo popover', async () => {
  let finish!: (value: Awaited<ReturnType<typeof api.getCodexPermissions>>) => void;
  vi.mocked(api.getCodexPermissions)
    .mockReturnValueOnce(new Promise(resolve => { finish = resolve; }))
    .mockResolvedValueOnce({ current: 'Approve for me', modes: [
      { numero: 1, nome: 'Approve for me', desc: '', cursor: true, atual: true },
      { numero: 2, nome: 'Full Access', desc: '', cursor: false, atual: false },
    ] });
  vi.mocked(api.setCodexPermission).mockResolvedValueOnce({ current: 'Full Access' });
  const props = await montar();
  (props as unknown as { headless: boolean }).headless = true;
  (props as unknown as { estreito: boolean }).estreito = true;
  await flush();
  document.querySelector<HTMLButtonElement>('.pill-duo button[aria-haspopup][aria-label="' + m.composer_permissao() + '"]')!.click();
  await flush();
  button('Full Access').click();
  await flush();
  expect(document.querySelector('.pill-duo button[aria-label="Full Access"]')).not.toBeNull();

  finish({ current: 'Approve for me', modes: [] });
  await flush();
  expect(document.querySelector('.pill-duo button[aria-label="Full Access"]')).not.toBeNull();
});

it('reconexão sem modo confirmado mostra Modo e Shift+Tab pede Planejar', async () => {
  const props = await montar();
  props.codexMode = null; await flush();
  expect(document.querySelector('[data-mode-trigger]')?.textContent?.trim()).toBe(m.chat_mode_label());
  document.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', {
    key: 'Tab', shiftKey: true, bubbles: true, cancelable: true,
  }));
  await flush();
  expect(api.setCodexMode).toHaveBeenLastCalledWith('codex-test', 'plan', undefined);
});

it('lista skills com barra e preenche argumentos antes do envio', async () => {
  const props = await montar();
  props.inputText = '/rev'; await flush();
  const skill = [...document.querySelectorAll('button')].find(b => b.textContent?.includes('/revisar'))!;
  expect(skill).toBeTruthy(); skill.click(); await flush();
  expect(document.querySelector('textarea')!.value).toBe('/revisar ');
  expect(props.onSend).not.toHaveBeenCalled();
});

it('setas percorrem as sugestões de comando', async () => {
  const props = await montar();
  props.inputText = '/re'; await flush();
  const textarea = document.querySelector('textarea')!;
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toContain('/revisar');

  textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  await flush();
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toContain('/resumir');

  textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true }));
  await flush();
  expect(document.querySelector('[role="option"][aria-selected="true"]')?.textContent).toContain('/revisar');
});

it('expõe a sugestão ativa ao leitor de tela', async () => {
  const props = await montar();
  props.inputText = '/re'; await flush();
  const textarea = document.querySelector('textarea')!;
  const listbox = document.querySelector<HTMLElement>('[role="listbox"]')!;
  let selected = document.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')!;

  expect(listbox.id).not.toBe('');
  expect(textarea.getAttribute('aria-controls')).toBe(listbox.id);
  expect(textarea.getAttribute('aria-activedescendant')).toBe(selected.id);

  textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
  await flush();
  selected = document.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')!;
  expect(textarea.getAttribute('aria-activedescendant')).toBe(selected.id);
});

it('Tab completa a sugestão selecionada', async () => {
  const props = await montar();
  props.inputText = '/resu'; await flush();
  const textarea = document.querySelector('textarea')!;
  const tab = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
  textarea.dispatchEvent(tab);
  await flush();

  expect(tab.defaultPrevented).toBe(true);
  expect(textarea.value).toBe('/resumir ');
  expect(document.activeElement).toBe(textarea);
});

it('Tab só completa a skill no Claude sem executá-la', async () => {
  const props = await montar();
  (props as unknown as { provider: 'claude' | 'codex' }).provider = 'claude';
  props.inputText = '/resu'; await flush();
  const textarea = document.querySelector('textarea')!;
  textarea.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }));
  await flush();

  expect(textarea.value).toBe('/resumir ');
  expect(props.onCommand).not.toHaveBeenCalled();
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

it('mostra o andamento de Orientar, bloqueia repetição e permite tentar após falha', async () => {
  const props = await montar();
  let finish!: () => void;
  props.onSteer.mockImplementationOnce(() => new Promise<boolean>(resolve => { finish = () => resolve(true); }));
  const steer = document.querySelector<HTMLButtonElement>('.fila-chip')!;
  steer.click(); steer.click(); await flush();
  expect(props.onSteer).toHaveBeenCalledTimes(1);
  expect(steer.disabled).toBe(true);
  expect(steer.textContent).toContain(m.askq_enviando());
  finish(); await flush();
  expect(steer.disabled).toBe(false);
  expect(document.querySelector('.steer-feedback')?.textContent).toBe(m.codex_orientar_recebido());
  props.onSteer.mockRejectedValueOnce(new Error('Turno encerrado'));
  steer.click(); await flush();
  expect(document.querySelector('.steer-feedback')).toBeNull();
  expect(document.querySelector('.send-error')?.textContent).toBe('Turno encerrado');
  expect(steer.disabled).toBe(false);
  steer.click(); await flush();
  expect(props.onSteer).toHaveBeenCalledTimes(3);
});

it('tira Orientar após aceite e só oferece de novo para uma fila nova', async () => {
  const props = await montar();
  let finish!: () => void;
  props.onSteer.mockImplementationOnce(() => new Promise<boolean>(resolve => { finish = () => resolve(true); }));
  document.querySelector<HTMLButtonElement>('.fila-chip')!.click(); await flush();
  props.filaCount = 0; await flush();
  expect(document.querySelector<HTMLButtonElement>('.fila-chip')?.disabled).toBe(true);
  finish(); await flush();
  expect(document.querySelector('.fila-chip')).toBeNull();
  expect(document.querySelector('.steer-feedback')?.textContent).toBe(m.codex_orientar_recebido());
  props.filaCount = 1; await flush();
  expect(document.querySelector<HTMLButtonElement>('.fila-chip')?.disabled).toBe(false);
});

it('resposta atrasada não mostra recibo em turno encerrado', async () => {
  const props = await montar();
  let finish!: () => void;
  props.onSteer.mockImplementationOnce(() => new Promise<boolean>(resolve => { finish = () => resolve(true); }));
  document.querySelector<HTMLButtonElement>('.fila-chip')!.click(); await flush();
  props.sessionState = 'idle'; await flush();
  finish(); await flush();
  expect(document.querySelector('.steer-feedback')).toBeNull();
});

it('não confirma recebimento quando nenhuma orientação foi encaminhada', async () => {
  const props = await montar();
  props.onSteer.mockResolvedValueOnce(false);
  document.querySelector<HTMLButtonElement>('.fila-chip')!.click(); await flush();
  expect(document.querySelector('.steer-feedback')?.textContent).toBe(m.codex_orientar_sem_envio());
});

it('Claude sem terminal oferece Orientar no envio e o chip da fila, como o Codex', async () => {
  const props = await montar();
  (props as unknown as { provider: string }).provider = 'claude';
  (props as unknown as { headless: boolean }).headless = true;
  await flush();
  expect(document.querySelector('button.fila-chip')).not.toBeNull();
  props.inputText = 'corrigir caminho'; await flush();
  const orient = [...document.querySelectorAll('button')].find(b => b.title === m.codex_orientar_ajuda());
  expect(orient).toBeDefined();
  orient!.click(); await flush();
  expect(props.onSend).toHaveBeenLastCalledWith('corrigir caminho', true);
});
