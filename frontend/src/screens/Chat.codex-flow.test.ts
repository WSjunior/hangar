// @vitest-environment happy-dom
// O trajeto usa Chat, Composer, MessageList e o stepper reais; só rede e painéis auxiliares são simulados.
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRawSnippet, mount, tick, unmount } from 'svelte';
import Chat from './Chat.svelte';
import * as api from '@hangar/core';
import * as m from '../paraglide/messages';
import { overwriteGetLocale } from '../paraglide/runtime';

function vazio() { return { default: createRawSnippet(() => ({ render: () => '<div />' })) }; }
vi.mock('../components/NavBar.svelte', vazio);
vi.mock('../components/SessionSwitcherSheet.svelte', vazio);
vi.mock('../components/CreateSessionSheet.svelte', vazio);
vi.mock('../components/UsageSheet.svelte', vazio);
vi.mock('../components/Git.svelte', vazio);
vi.mock('../components/PreviewSheet.svelte', vazio);
vi.mock('../components/ActivitySheet.svelte', vazio);
vi.mock('../components/TerminalMirror.svelte', vazio);
vi.mock('../components/RunSheet.svelte', vazio);
vi.mock('../components/MoreSheet.svelte', vazio);
vi.mock('../components/AttachmentsSheet.svelte', vazio);
vi.mock('../components/CodexLimitsSheet.svelte', vazio);
vi.mock('../components/ForwardSheet.svelte', vazio);
vi.mock('../components/PairSheet.svelte', vazio);
vi.mock('../components/PairChatModal.svelte', vazio);
vi.mock('../components/LoopSheet.svelte', vazio);
vi.mock('../components/DesktopSessionContext.svelte', vazio);
vi.mock('../components/files/FileViewer.svelte', vazio);
vi.mock('../lib/aquecimento', () => ({
  aoAquecer: () => Promise.resolve(), segurarAquecimento: vi.fn(), soltarAquecimento: vi.fn(),
}));
vi.mock('../lib/ttsPlayer.svelte', () => ({ ttsPlayer: { active: false, loading: false } }));
vi.mock('../lib/auth', () => ({
  listServers: () => [{ id: 'codex-flow', label: 'Teste', baseUrl: 'http://teste', token: 'teste' }],
  getActiveId: () => 'codex-flow',
}));

const sse = vi.hoisted(() => ({ handlers: new Map<string, (event: MessageEvent) => void>() }));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof api>(),
  getHistory: vi.fn().mockResolvedValue([]),
  getHistoryDesde: vi.fn().mockResolvedValue({ eventos: [], etag: null }),
  getSessions: vi.fn().mockResolvedValue([{ name: 'codex-flow', provider: 'codex', tracked: true, state: 'working', cwd: '/teste' }]),
  getRunners: vi.fn().mockResolvedValue({ running: false }),
  getPlan: vi.fn().mockResolvedValue(null),
  getWorkflows: vi.fn().mockResolvedValue([]),
  getCommands: vi.fn().mockResolvedValue([]),
  getCodexModels: vi.fn().mockResolvedValue({ models: [], current: { model: 'gpt-6-astra', effort: 'high', mode: 'default' } }),
  sendInput: vi.fn().mockResolvedValue({ ok: true, queued: true }),
  steerSession: vi.fn().mockResolvedValue({ ok: true, promoted: true }),
  broadcast: vi.fn().mockResolvedValue({}),
  setCodexMode: vi.fn().mockResolvedValue({ model: 'gpt-6-astra', effort: 'high', mode: 'default' }),
  answerQuestions: vi.fn().mockResolvedValue({ ok: true }),
  openEventStream: vi.fn(() => ({
    onmessage: null, onerror: null, close: vi.fn(), readyState: 1,
    addEventListener: (type: string, handler: (event: MessageEvent) => void) => sse.handlers.set(type, handler),
  })),
}));

let components: ReturnType<typeof mount>[] = [];
async function flush() { await tick(); await new Promise(resolve => setTimeout(resolve, 0)); await tick(); }
async function emit(type: string, data: unknown) {
  expect(sse.handlers.has(type)).toBe(true);
  sse.handlers.get(type)!({ data: JSON.stringify(data) } as MessageEvent);
  await flush();
}
function bolhas(text: string) {
  return [...document.querySelectorAll('.bubble-text')].filter(el => el.textContent === text);
}
function orientar() { return document.querySelector<HTMLButtonElement>('button.fila-chip'); }
async function montar() {
  const target = document.createElement('div'); document.body.appendChild(target);
  components.push(mount(Chat, { target, props: {
    sessionName: 'codex-flow', desktop: true, showContextPanel: false,
    onBack: vi.fn(), onNavigateToChat: vi.fn(),
  } }));
  await flush(); await flush();
  await emit('state', { session: 'codex-flow', state: 'working', codex_mode: 'default' });
}
async function enfileirar(text: string) {
  const textarea = document.querySelector<HTMLTextAreaElement>('textarea')!;
  textarea.value = text;
  textarea.dispatchEvent(new Event('input', { bubbles: true }));
  await flush();
  document.querySelector<HTMLButtonElement>('button.send-btn')!.click();
  await flush();
  expect(api.sendInput).toHaveBeenLastCalledWith('codex-flow', text);
  expect(textarea.value).toBe('');
  expect(bolhas(text)).toHaveLength(1);
  await emit('message', { id: 'queued-fila', kind: 'user_msg', text, ts: '2026-09-07T12:00:00Z' });
  expect(bolhas(text)).toHaveLength(1);
  expect(orientar()).not.toBeNull();
}

beforeEach(() => {
  vi.clearAllMocks(); sse.handlers.clear();
  const storage = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
    removeItem: (key: string) => storage.delete(key), clear: () => storage.clear(),
  });
  overwriteGetLocale(() => 'pt');
  vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}', { status: 200 }));
});
afterEach(async () => {
  for (const component of components) await unmount(component);
  components = []; document.body.innerHTML = ''; vi.restoreAllMocks(); vi.unstubAllGlobals();
});

it('digita, enfileira, orienta e reconcilia a mensagem definitiva sem duplicação', async () => {
  await montar();
  await enfileirar('Preserve a configuração atual');
  orientar()!.click(); await flush();
  expect(api.steerSession).toHaveBeenLastCalledWith('codex-flow');
  await emit('message', { id: 'mensagem-real', kind: 'user_msg', text: 'Preserve a configuração atual', ts: '2026-09-07T12:00:01Z' });
  expect(bolhas('Preserve a configuração atual')).toHaveLength(1);
  expect(orientar()).toBeNull();
});

it('uma falha ao orientar preserva a bolha da fila e permite tentar novamente', async () => {
  await montar(); await enfileirar('Mantenha esta orientação');
  vi.mocked(api.steerSession).mockRejectedValueOnce(new Error('O turno terminou'));
  orientar()!.click(); await flush();
  expect(bolhas('Mantenha esta orientação')).toHaveLength(1);
  expect(orientar()).not.toBeNull();
  expect(document.body.textContent).toContain('O turno terminou');
});

it('uma pergunta SSE abre o stepper real e a resolução no terminal fecha o cartão', async () => {
  await montar();
  await emit('ask_question', { provider: 'codex', request_id: 7, questions: [{
    id: 'destino', header: 'Destino', question: 'Onde aplicar a alteração?', multiSelect: false,
    isOther: true, options: [{ label: 'Nesta máquina', description: 'Preservar o servidor' }],
  }] });
  expect(document.body.textContent).toContain('Onde aplicar a alteração?');
  expect([...document.querySelectorAll('button')].some(el => el.textContent?.includes('Nesta máquina'))).toBe(true);
  await emit('ask_question', null);
  expect(document.body.textContent).not.toContain('Onde aplicar a alteração?');
  expect(api.answerQuestions).not.toHaveBeenCalled();
});

it('envia respostas de várias perguntas pelo request_id e conserva o formulário quando a API falha', async () => {
  await montar();
  await emit('ask_question', { provider: 'codex', request_id: 0, questions: [
    { id: 'destino', header: 'Destino', question: 'Onde aplicar?', multiSelect: false,
      options: [{ label: 'Localmente', description: '' }] },
    { id: 'nome', header: 'Nome', question: 'Qual nome?', multiSelect: false, options: [] },
  ] });
  document.querySelector<HTMLButtonElement>('.option-btn')!.click(); await flush();
  const input = document.querySelector<HTMLInputElement>('.ask-card input')!;
  expect(input).not.toBeNull();
  input.value = 'João'; input.dispatchEvent(new Event('input', { bubbles: true })); await flush();
  document.querySelector<HTMLButtonElement>('.text-actions .primary-btn')!.click(); await flush();
  vi.mocked(api.answerQuestions).mockRejectedValueOnce(new Error('Conexão interrompida'));
  document.querySelector<HTMLButtonElement>('.ask-card .primary-btn')!.click(); await flush();
  expect(api.answerQuestions).toHaveBeenLastCalledWith('codex-flow', [
    expect.objectContaining({ question_id: 'destino', kind: 'option', indices: [0] }),
    expect.objectContaining({ question_id: 'nome', kind: 'text', value: 'João' }),
  ], 0);
  expect(document.body.textContent).toContain('Conexão interrompida');
  expect(document.body.textContent).toContain('João');
  document.querySelector<HTMLButtonElement>('.ask-card .primary-btn')!.click(); await flush();
  expect(api.answerQuestions).toHaveBeenCalledTimes(2);
  expect(document.querySelector('.ask-card')).toBeNull();
});

it('aprovar o plano envia uma única vez à sessão atual mesmo com mandar pros dois ligado', async () => {
  const sessions = await api.getSessions();
  vi.mocked(api.getSessions).mockResolvedValueOnce(sessions.map(session => ({ ...session, pair_peers: ['par'] })));
  type Mode = Awaited<ReturnType<typeof api.setCodexMode>>;
  let resolveMode!: (value: Mode) => void;
  const mode = {
    promise: new Promise<Mode>(resolve => { resolveMode = resolve; }),
    resolve: (value: Mode) => resolveMode(value),
  };
  vi.mocked(api.setCodexMode).mockReturnValueOnce(mode.promise);
  await montar();
  const both = document.querySelector<HTMLButtonElement>('button.both-chip')!;
  expect(both).not.toBeNull();
  both.click(); await flush();
  expect(both.getAttribute('aria-pressed')).toBe('true');
  await emit('message', {
    id: 'plano-completo', kind: 'assistant_msg', ts: '2026-09-07T12:00:00Z',
    text: '<proposed_plan>\n## Plano\nPreservar a configuração e validar a alteração.\n</proposed_plan>',
  });
  await emit('state', { session: 'codex-flow', state: 'idle', codex_mode: 'plan' });
  const implement = document.querySelector<HTMLButtonElement>('.plan-actions .primary-btn')!;
  expect(implement.textContent).toContain(m.chat_plan_implementar());
  expect(implement.disabled).toBe(false);
  // Dois cliques no mesmo quadro precisam compartilhar o envio ainda em curso.
  implement.click(); implement.click(); await flush();
  expect(api.setCodexMode).toHaveBeenCalledExactlyOnceWith('codex-flow', 'default');
  expect(api.sendInput).not.toHaveBeenCalled();
  mode.resolve({ model: 'gpt-6-astra', effort: 'high', mode: 'default' });
  await flush();
  expect(api.sendInput).toHaveBeenCalledExactlyOnceWith('codex-flow', m.chat_plan_pedido());
  expect(api.broadcast).not.toHaveBeenCalled();
  expect(both.getAttribute('aria-pressed')).toBe('true');
});
