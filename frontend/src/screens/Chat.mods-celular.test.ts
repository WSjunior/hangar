// @vitest-environment happy-dom
// Task 8: no celular a interface dos mods (faixa e painéis) fica oculta até ligar no menu "⋯", e com ela
// oculta o app não chama nenhuma rota de mod. Mesmo molde de mocks do Chat.test.ts: filhos pesados em stub.
import { it, expect, vi, beforeEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import { createRawSnippet } from 'svelte';
import Chat from './Chat.svelte';
import { chamadasFocus } from './ComposerStub.svelte';
import { modsCelular } from '../lib/modsCelular.svelte';
import { overwriteGetLocale } from '../paraglide/runtime';
import * as m from '../paraglide/messages';
import GitTabs from '../components/git/GitTabs.svelte';
import { createGitStore } from '../lib/gitStore.svelte';
import { renderMarkdown } from '../lib/markdown';
import type { AggSession, SessionInfo } from '@hangar/core';
import * as diag from '../lib/diag';
import { getBaseUrl } from '../lib/auth';
const janelaCtl = vi.hoisted(() => ({ desktop: false }));
vi.mock('../lib/desktop.svelte', () => ({ desktop: { get atual() { return janelaCtl.desktop; } } }));
vi.mock('../lib/diag', () => ({ registrar: vi.fn(), novoReq: () => 'stream-teste' }));

// Stub de componente Svelte 5: createRawSnippet é o padrão do DesktopShell.test.ts — uma classe
// com $destroy (padrão Svelte 4) quebra no mount do Svelte 5 com "cannot be invoked without new".
function stubDe() { return { default: createRawSnippet(() => ({ render: () => '<div />' })) }; }
// O FileViewer vira um stub com o botão .fechar REAL: é ele que o Chat foca ao abrir o visor.
function stubVisor() {
  return { default: createRawSnippet(() => ({ render: () => '<div class="visor" role="region" tabindex="-1"><button class="fechar">×</button></div>' })) };
}

vi.mock('../components/NavBar.svelte', stubDe);
vi.mock('../components/MessageList.svelte', stubDe);
vi.mock('../components/Composer.svelte', async () => ({
  default: (await import('./ComposerStub.svelte')).default,
}));
vi.mock('../components/SessionSwitcherSheet.svelte', stubDe);
vi.mock('../components/CreateSessionSheet.svelte', stubDe);
vi.mock('../components/UsageSheet.svelte', stubDe);
vi.mock('../components/Git.svelte', stubDe);
vi.mock('../components/PreviewSheet.svelte', stubDe);
vi.mock('../components/ActivitySheet.svelte', stubDe);
vi.mock('../components/TerminalMirror.svelte', stubDe);
vi.mock('../components/AskQuestionSheet.svelte', stubDe);
vi.mock('../components/RunSheet.svelte', stubDe);
vi.mock('../components/MoreSheet.svelte', stubDe);
vi.mock('../components/AttachmentsSheet.svelte', stubDe);
vi.mock('../components/CodexLimitsSheet.svelte', stubDe);
vi.mock('../components/ForwardSheet.svelte', stubDe);
vi.mock('../components/PairSheet.svelte', stubDe);
vi.mock('../components/PairChatModal.svelte', stubDe);
vi.mock('../components/LoopSheet.svelte', stubDe);
vi.mock('../components/DesktopSessionContext.svelte', stubDe);
vi.mock('../components/files/FileViewer.svelte', stubVisor);

// Controle do SSE mockado: o Chat conecta via openEventStream().addEventListener('state', ...)
// — para montar uma sessao morta (B5 rodada 4) o teste precisa disparar o evento na mao.
const sseCtl = vi.hoisted(() => {
  const handlers = new Map<string, (e: MessageEvent) => void>();
  return {
    handlers,
    state: (dados: unknown) => {
      const h = handlers.get('state');
      if (h) h({ data: JSON.stringify(dados) } as MessageEvent);
    },
  };
});
const sessionsStoreCtl = vi.hoisted(() => ({ rows: [] as unknown[], byServer: [] as unknown[] }));

// API: só o que o mount do Chat toca precisa responder; o resto nunca chega a ser chamado
// com os filhos stubados.
vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()),
  pressPluginButton: vi.fn(async () => ({ ok: true })),
  showPluginPane: vi.fn(async () => ({ ok: true })),
  inputPluginField: vi.fn(async () => ({ ok: true })),
  getPermissionModes: vi.fn().mockResolvedValue({ current: 'plan', modes: ['plan', 'auto', 'manual', 'acceptEdits'] }),
  setPermissionMode: vi.fn().mockResolvedValue({ mode: 'plan', current: 'plan' }),
  isTimeoutError: vi.fn(() => false),
  errorDetail: vi.fn(async () => ''),
  getHistory: vi.fn(async () => []),
  getHistoryDesde: vi.fn(async () => ({ eventos: [], etag: null })),
  openEventStream: vi.fn(() => ({
    onmessage: () => {}, onerror: () => {}, close: () => {},
    readyState: 0,
    addEventListener: (tipo: string, h: (e: MessageEvent) => void) => { sseCtl.handlers.set(tipo, h); },
  })),
  // o teste integrado do B1 (rodada 5) monta o GitTabs real: as funcoes do FilesPanel
  listFiles: vi.fn(async () => ({
    entries: [{ name: 'a.txt', path: 'a.txt', is_dir: false, size: 1, changed: null, add: 0, del: 0 }],
    truncated: false,
  })),
  readFile: vi.fn(async () => ({ path: 'a.txt', text: 'A', size: 1, truncated: false })),
  searchFiles: vi.fn(async () => ({ hits: [], truncated: false, mode: 'names' })),
  resolverCitados: vi.fn(async () => ({ ok: { '/repo/a.txt': { relativo: 'a.txt', real: '/repo/a.txt' } }, faltam: [] })),
  fileUrl: vi.fn((name: string, path: string) => `/api/sessions/${name}/file?path=${encodeURIComponent(path)}`),
  pathDiff: vi.fn(async () => ({
    path: 'a.txt', diff: '', truncated: false,
    escopo_pedido: 'branch', escopo_usado: 'branch', base: null, motivo: null,
  })),
  getSessions: vi.fn(async () => []),
  getRunners: vi.fn(async () => ({ running: false })),
  getPlan: vi.fn(async () => null),
  getWorkflows: vi.fn(async () => []),
  sendInput: vi.fn(),
  steerSession: vi.fn(),
  broadcast: vi.fn(),
  selectOption: vi.fn(),
  interrupt: vi.fn(),
  createSession: vi.fn(),
  answerQuestions: vi.fn(),
  isAbortError: vi.fn(() => false),
 loopBadge: vi.fn(() => null), LOOP_TONE_COLOR: {},
 parseStatusLine: vi.fn(() => null),
 appendTail: vi.fn(), hasSeam: vi.fn(), prependOlder: vi.fn(),
  createActivityFolder: vi.fn(() => ({
    snapshot: () => ({ tasks: [], inProgress: 0, running: 0, agents: [], shells: [], runningAgents: 0, runningShells: 0, writeEvents: [] }),
    push: () => {}, save: () => {}, attach: () => {}, reset: () => {},
  })),
 formataErro: vi.fn(() => ''),
}));
vi.mock('../lib/auth', () => ({
  listServers: vi.fn(() => [{ id: 'srv-test', label: 'T', baseUrl: 'http://x', token: 't' }]),
  listAllServers: vi.fn(() => [{ id: 'srv-test', label: 'T', baseUrl: 'http://x', token: 't' }]),
  listOwnServers: vi.fn(() => [{ id: 'srv-test', label: 'T', baseUrl: 'http://x', token: 't' }]),
  getActiveId: vi.fn(() => 'srv-test'),
  getBaseUrl: vi.fn(() => 'http://x'),
  isActiveInvite: vi.fn(() => false),
}));
vi.mock('../lib/sessionsStore.svelte', () => ({
  sessionsStore: {
    sessionsForServer(id: string) { return (sessionsStoreCtl.rows as AggSession[]).filter(s => s.serverId === id); },
    get rows() { return sessionsStoreCtl.rows; },
    get byServer() { return sessionsStoreCtl.byServer; },
  },
}));
vi.mock('../lib/ttsPlayer.svelte', () => ({ ttsPlayer: { active: false, loading: false } }));
vi.mock('../lib/ouvir', () => ({ ouvirTexto: vi.fn() }));
vi.mock('../lib/speakable', () => ({ textoFalavelComCodigo: vi.fn(() => '') }));
// O módulo inteiro é mockado vazio; a exceção é a função PURA que o Chat usa pra chavear o
// navegador embutido por sessão — duplicada aqui pra não puxar o módulo real (e as deps dele).
vi.mock('../lib/workspaceCommands', () => ({
  workspaceSessionKey: (s: { serverId: string; name: string }) => `${s.serverId}::${s.name}`,
}) );

function montar() {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const comp = mount(Chat, {
    target: el,
    props: { sessionName: 'sess', onBack: vi.fn(), onNavigateToChat: vi.fn(), desktop: false, showContextPanel: false },
  });
  return { el, comp: comp as never };
}

const botao = (key: string, label: string) => ({
  type: 'Button', props: { label, key }, children: [],
});
const FAIXA = { type: 'Box', children: [botao('b1', 'Abrir mod')] };
const PAINEL = { id: 'p1', title: 'Painel um', placement: 'inline', columns: 40, tree: { type: 'Box', children: [botao('b2', 'No painel')] } };

function emitirPluginUi() {
  sseCtl.handlers.get('plugin_ui')?.({ data: JSON.stringify({ above: FAIXA, panes: [PAINEL], source: 'surface' }) } as MessageEvent);
}

beforeEach(async () => {
  overwriteGetLocale(() => 'pt');
  document.body.innerHTML = '';
  sessionsStoreCtl.rows = [];
  sessionsStoreCtl.byServer = [];
  janelaCtl.desktop = false;
  modsCelular.ligado = false;
  const api = await import('@hangar/core');
  vi.mocked(api.pressPluginButton).mockClear();
  vi.mocked(api.showPluginPane).mockClear();
  vi.mocked(api.inputPluginField).mockClear();
});

async function chamadasDeMod() {
  const api = await import('@hangar/core');
  const chamadas: unknown[][] = [];
  for (const f of [api.pressPluginButton, api.showPluginPane, api.inputPluginField]) chamadas.push(...vi.mocked(f).mock.calls);
  return chamadas;
}

it('celular com a preferência desligada: faixa e painel não são desenhados e nenhuma rota de mod é chamada', async () => {
  const t = montar();
  try {
    await tick();
    emitirPluginUi();
    await tick();
    expect(t.el.querySelector('.plugin-band')).toBeNull();
    expect(t.el.querySelector('.plugin-pane')).toBeNull();
    expect(t.el.textContent).not.toContain('Abrir mod');
    expect(t.el.textContent).not.toContain('No painel');
    expect(await chamadasDeMod()).toEqual([]);
  } finally {
    await unmount(t.comp);
  }
});

it('celular: ligar a preferência desenha a faixa e o painel, e o clique chama a rota; desligar tira tudo na hora', async () => {
  const t = montar();
  try {
    await tick();
    emitirPluginUi();
    modsCelular.ligado = true;
    await tick();
    const abrir = [...t.el.querySelectorAll('button.button')].find((b) => b.textContent === 'Abrir mod') as HTMLElement;
    expect(abrir).toBeTruthy();
    expect(t.el.textContent).toContain('No painel');
    abrir.click();
    await tick();
    expect(await chamadasDeMod()).toHaveLength(1);
    modsCelular.ligado = false;
    await tick();
    expect(t.el.textContent).not.toContain('Abrir mod');
    expect(t.el.textContent).not.toContain('No painel');
  } finally {
    await unmount(t.comp);
  }
});

it('desktop: a interface dos mods aparece mesmo com a preferência do celular desligada', async () => {
  janelaCtl.desktop = true;
  const t = montar();
  try {
    await tick();
    emitirPluginUi();
    await tick();
    expect(t.el.textContent).toContain('Abrir mod');
    expect(t.el.textContent).toContain('No painel');
  } finally {
    await unmount(t.comp);
  }
});

it('celular com a interface oculta: o aviso (toast) do mod continua aparecendo', async () => {
  const t = montar();
  try {
    await tick();
    sseCtl.handlers.get('plugin_toast')?.({ data: JSON.stringify({ id: 't1', text: 'Aviso do mod', timeoutMs: 60000 }) } as MessageEvent);
    emitirPluginUi();
    await tick();
    expect(t.el.textContent).toContain('Aviso do mod');
    expect(t.el.textContent).not.toContain('Abrir mod');
  } finally {
    await unmount(t.comp);
  }
});

it('troca de aba recusada: código conhecido mostra a frase dele em qualquer status; 5xx sem código, a frase do app', async () => {
  janelaCtl.desktop = true;
  const api = await import('@hangar/core');
  const t = montar();
  try {
    await tick();
    const segundo = { ...PAINEL, id: 'p2', title: 'Painel dois' };
    sseCtl.handlers.get('plugin_ui')?.({ data: JSON.stringify({ above: FAIXA, panes: [PAINEL, segundo], shown_id: 'p1', source: 'surface' }) } as MessageEvent);
    await tick();
    const aba = () => [...t.el.querySelectorAll('button[role="tab"]')].find((b) => b.textContent === 'Painel dois') as HTMLElement;
    const aviso = () => t.el.querySelector('.plugin-band .notice')?.textContent?.trim();
    // 503 do dono único com `erro_mod_guarda_indisponivel`: a frase do código, não a genérica da aba.
    vi.mocked(api.showPluginPane).mockRejectedValueOnce(Object.assign(new Error('503: motivo — erro_mod_guarda_indisponivel'),
      { status: 503, code: 'erro_mod_guarda_indisponivel', envelope: { code: 'erro_mod_guarda_indisponivel', params: { motivo: 'x' }, msg: 'x' } }));
    aba().click();
    // A frase sai do core, no idioma dele: a mesma tabela de códigos (`mensagemDeErro`) que o app usa.
    await vi.waitFor(() => expect(aviso()).toBe(api.mensagemDeErro('erro_mod_guarda_indisponivel')));
    // 500 sem código: a frase genérica da troca de aba.
    vi.mocked(api.showPluginPane).mockRejectedValueOnce(Object.assign(new Error('500: Internal Server Error'), { status: 500 }));
    aba().click();
    await vi.waitFor(() => expect(aviso()).toBe(m.plugin_aba_falhou()));
  } finally {
    await unmount(t.comp);
  }
});
