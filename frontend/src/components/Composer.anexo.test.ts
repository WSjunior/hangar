// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Composer from './Composer.svelte';
import { configureApi, transcribeUploaded, uploadFile } from '@hangar/core';
import { dictations } from '../lib/dictationStore.svelte';

vi.mock('@hangar/core', async (orig) => ({
  ...(await orig<typeof import('@hangar/core')>()),
  getPermissionModes: vi.fn(async () => ({ current: 'plan', modes: ['plan'] })),
  getCommands: vi.fn(async () => []),
  getModelOptions: vi.fn(async () => []),
  uploadFile: vi.fn(),
  transcribeUploaded: vi.fn(async () => ({ path: '/up/p/s/antigo.webm', text: 'de novo', raw: 'de novo' })),
}));
vi.mock('../lib/sessionsStore.svelte', () => ({
  sessionsStore: { epoca: () => 0, retain: vi.fn(), release: vi.fn(), sessionsForServer: () => [] },
}));

const flush = async () => { await tick(); await new Promise((r) => setTimeout(r, 0)); await tick(); };

beforeEach(() => {
  dictations._resetForTests();
  localStorage.clear();
  vi.clearAllMocks();
  configureApi({ getBaseUrl: () => 'http://h', getToken: () => 't', onUnauthorized: () => {}, origin: null,
    createEventSource: () => { throw new Error('sem SSE no teste'); } });
});

it('áudio da galeria transcreve pelo nome, sem subir bytes, e a barra toca do servidor', async () => {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const comp = mount(Composer, {
    target,
    props: { sessionName: 's', sessionState: 'idle', status: null, inputText: '', sessionJsonl: 'j1',
      onSend: vi.fn(), onCommand: vi.fn(), onInterrupt: vi.fn(), onOpenGit: vi.fn(), onOpenPreview: vi.fn() },
  }) as unknown as { ditarAnexo: (a: string) => void };
  comp.ditarAnexo('antigo.webm');
  await flush();
  expect(transcribeUploaded).toHaveBeenCalledWith('s', 'antigo.webm', expect.objectContaining({ limpar: true }), undefined);
  expect(uploadFile).not.toHaveBeenCalled();
  expect(target.querySelector('textarea')!.value).toBe('de novo');
  expect(target.querySelector<HTMLAudioElement>('.ditado-audio')!.src).toContain('/uploads/antigo.webm');
  unmount(comp as never);
});
