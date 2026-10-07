// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Composer from './Composer.svelte';
import BoardCard from './BoardCard.svelte';
import type { BoardRow } from '../screens/Board.svelte';
import * as m from '../paraglide/messages';
import { uploadFile, transcribeFileForServer } from '@hangar/core';
import { dictations } from '../lib/dictationStore.svelte';

// Nos casos "stop"/"unmount" o upload nunca resolve: o `retain()` real deixaria a lista de sessões
// assinada entre os testes.
vi.mock('../lib/sessionsStore.svelte', () => ({
  sessionsStore: { epoca: () => 0, retain: vi.fn(), release: vi.fn(), sessionsForServer: () => [] },
}));
vi.mock('@hangar/core', async (original) => ({
  ...(await original<typeof import('@hangar/core')>()),
  getHistoryTailCached: vi.fn(async () => ({ evs: [], at: 0 })),
  getHistoryTailForServer: vi.fn(async () => []),
  fetchCotacao: vi.fn(async () => null),
  getPermissionModes: vi.fn(async () => ({ current: 'plan', modes: ['plan'] })),
  getCommands: vi.fn(async () => []),
  getModelOptions: vi.fn(async () => []),
  uploadFile: vi.fn(() => new Promise(() => {})),
  transcribeFileForServer: vi.fn(() => new Promise(() => {})),
}));

class FakeRecorder {
  static instances: FakeRecorder[] = [];
  static failStart = false;
  state = 'inactive';
  mimeType = 'audio/webm';
  ondataavailable: ((event: { data: Blob }) => void) | null = null;
  onstop: (() => void) | null = null;
  onerror: ((event: { error: Error }) => void) | null = null;
  constructor() { FakeRecorder.instances.push(this); }
  start() {
    if (FakeRecorder.failStart) throw new Error('start failed');
    this.state = 'recording';
  }
  stop() {
    this.state = 'inactive';
    queueMicrotask(() => {
      this.ondataavailable?.({ data: new Blob(['audio']) });
      this.onstop?.();
    });
  }
  fail() {
    this.onerror?.({ error: new Error('capture failed') });
    this.stop();
  }
}

const microphones: { tracks: { stop(): void }[] }[] = [];

function microphone() {
  const tracks = Array.from({ length: 2 }, () => ({
    readyState: 'live', enabled: true,
    stop() { this.readyState = 'ended'; },
  }));
  const mic = { tracks, stream: { getTracks: () => tracks } as unknown as MediaStream };
  microphones.push(mic);
  return mic;
}

let cleanup: (() => Promise<void>) | undefined;
const getUserMedia = vi.fn();
const flush = async () => { await tick(); await new Promise((resolve) => setTimeout(resolve, 0)); await tick(); };

beforeEach(() => {
  dictations._resetForTests();
  localStorage.clear();
  vi.clearAllMocks();
  FakeRecorder.instances = [];
  FakeRecorder.failStart = false;
  vi.stubGlobal('MediaRecorder', FakeRecorder);
  Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } });
  vi.stubGlobal('AudioContext', class {
    state = 'running';
    close() { return Promise.resolve(); }
    resume() { return Promise.resolve(); }
    createAnalyser() { return { fftSize: 256, frequencyBinCount: 128, getByteTimeDomainData() {} }; }
    createMediaStreamSource() { return { connect() {} }; }
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});

afterEach(async () => {
  await cleanup?.();
  cleanup = undefined;
  microphones.splice(0).forEach(({ tracks }) => tracks.forEach((track) => track.stop()));
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function render(kind: 'Composer' | 'BoardCard') {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const common = { sessionName: 'mic', sessionState: 'idle' as const, status: null,
    inputText: '', onSend: vi.fn(), onCommand: vi.fn(), onInterrupt: vi.fn(), onOpenGit: vi.fn(), onOpenPreview: vi.fn() };
  const component = kind === 'Composer'
    ? mount(Composer, { target, props: common })
    : mount(BoardCard, { target, props: {
      session: { name: 'mic', serverId: 'srv', state: 'idle', provider: 'claude' } as BoardRow,
      server: { id: 'srv', label: 'S', baseUrl: 'http://x', token: 't' }, color: '#fff',
      draft: '', onDraftChange: vi.fn(), pending: [], updatePending: vi.fn(),
      sendError: '', onSendError: vi.fn(), onOpen: vi.fn(),
    } });
  cleanup = async () => { await unmount(component); target.remove(); };
  await flush();
  const clickMic = () => target.querySelector<HTMLButtonElement>(
    kind === 'Composer' ? '.mic-btn' : `button[aria-label="${m.board_gravar_audio()}"], button[aria-label="${m.composer_cancelar_prep_mic()}"]`,
  )!.click();
  return { clickMic, destroy: async () => { await cleanup?.(); cleanup = undefined; } };
}

describe.each(['Composer', 'BoardCard'] as const)('%s libera o microfone', (kind) => {
  it.each(['stop', 'error', 'unmount', 'start failure'])('%s encerra todas as tracks', async (ending) => {
    const { tracks, stream } = microphone();
    getUserMedia.mockResolvedValue(stream);
    FakeRecorder.failStart = ending === 'start failure';
    const { clickMic, destroy } = await render(kind);
    clickMic();
    await flush();
    if (ending === 'stop') clickMic();
    if (ending === 'error') FakeRecorder.instances[0].fail();
    if (ending === 'unmount') await destroy();
    await flush();
    expect(tracks.map((track) => track.readyState)).toEqual(['ended', 'ended']);
    // Desmontar o Composer GRAVANDO (trocar de sessão) sobe o áudio pra origem; o card não muda.
    if (kind === 'Composer') {
      expect(uploadFile).toHaveBeenCalledTimes(ending === 'stop' || ending === 'unmount' ? 1 : 0);
    } else {
      expect(transcribeFileForServer).toHaveBeenCalledTimes(ending === 'stop' ? 1 : 0);
    }
  });

  it.each(['cancel', 'unmount'])('permissão concedida após %s não inicia captura', async (ending) => {
    const { tracks, stream } = microphone();
    let allow!: (stream: MediaStream) => void;
    getUserMedia.mockReturnValue(new Promise<MediaStream>((resolve) => { allow = resolve; }));
    const { clickMic, destroy } = await render(kind);
    clickMic();
    await flush();
    if (ending === 'cancel') clickMic();
    else await destroy();
    allow(stream);
    await flush();
    expect(tracks.map((track) => track.readyState)).toEqual(['ended', 'ended']);
    expect(FakeRecorder.instances).toHaveLength(0);
  });
});
