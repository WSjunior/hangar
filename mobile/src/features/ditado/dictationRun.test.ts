import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DictationDraft } from '../../stores/drafts';

const { memory, core } = vi.hoisted(() => ({ memory: new Map<string, string>(), core: { transcribe: vi.fn(), upload: vi.fn() } }));
vi.mock('react-native-mmkv', () => ({
  createMMKV: () => ({
    getString: (key: string) => memory.get(key),
    set: (key: string, value: string) => { memory.set(key, value); },
    remove: (key: string) => { memory.delete(key); },
  }),
}));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  transcribeUploadedForServer: core.transcribe,
  uploadFileForServer: core.upload,
}));
vi.mock('../../stores/chat', () => ({ chatStore: () => ({ use: { setState: () => {} } }) }));

import { readDictation, writeDictation } from '../../stores/drafts';
import { dictationAudio, runDictation } from './dictationRun';

const server = { id: 's1', label: 'A', baseUrl: 'https://a.test', token: 't' };
const voice = (patch: Partial<DictationDraft> = {}): DictationDraft => ({
  version: 1, id: 'v1', audio: { uri: 'file:///doc/draft-attachments/1-1.m4a', name: 'd.m4a', mime: 'audio/m4a', kind: 'file' },
  transcript: null, draftRevision: 0, before: '', motivo: 'botao', status: 'pending', text: '', raw: '', issue: '', ...patch,
});

describe('runDictation', () => {
  beforeEach(() => { memory.clear(); core.transcribe.mockReset(); core.upload.mockReset(); });

  it('sobe o áudio e guarda o caminho antes de transcrever; a falha da transcrição o mantém', async () => {
    core.upload.mockResolvedValueOnce({ path: '/up/sess/d.m4a' });
    let savedBefore: string | undefined;
    core.transcribe.mockImplementationOnce(async () => {
      savedBefore = readDictation('s1', 'sess')?.serverPath;
      throw new Error('502: groq');
    });
    writeDictation('s1', 'sess', voice());
    await expect(runDictation(server, 's1', 'sess', voice(), { file: new File(['a'], 'd.m4a') }, { isActive: () => false }))
      .rejects.toMatchObject({ message: '502: groq', lost: false });
    expect(savedBefore).toBe('/up/sess/d.m4a');
    expect(core.upload).toHaveBeenCalledExactlyOnceWith(server, 'sess', expect.any(File), { audioOnly: true });
    expect(core.transcribe).toHaveBeenCalledExactlyOnceWith(server, 'sess', 'd.m4a', { limpar: true, estilo: undefined });
    expect(readDictation('s1', 'sess')).toMatchObject({ status: 'failed', serverPath: '/up/sess/d.m4a', issue: '502: groq' });
  });

  it('"de novo" pelo caminho guardado não sobe outra cópia; sem estilo_aplicado vale o estilo pedido', async () => {
    core.transcribe.mockResolvedValueOnce({ path: '/up/sess/d.m4a', text: 'ok', raw: 'ok cru', aviso: null });
    writeDictation('s1', 'sess', voice({ audio: null, estilo: 'limpar' }));
    await runDictation(server, 's1', 'sess', voice({ audio: null, estilo: 'limpar' }), { arquivo: '/up/sess/d.m4a' }, { isActive: () => false });
    expect(core.upload).not.toHaveBeenCalled();
    expect(core.transcribe).toHaveBeenCalledExactlyOnceWith(server, 'sess', '/up/sess/d.m4a', { limpar: true, estilo: 'limpar' });
    expect(readDictation('s1', 'sess')).toMatchObject({ status: 'ready', serverPath: '/up/sess/d.m4a', applied: 'limpar' });
  });

  it('"de novo" pelo nome solto, sem `path` na resposta, mantém o caminho absoluto guardado', async () => {
    core.transcribe.mockResolvedValueOnce({ path: '', text: 'ok', raw: 'ok', aviso: null });
    const v = voice({ serverPath: '/up/sess/d.m4a' });
    writeDictation('s1', 'sess', v);
    await runDictation(server, 's1', 'sess', v, { arquivo: 'd.m4a' }, { isActive: () => false });
    expect(readDictation('s1', 'sess')?.serverPath).toBe('/up/sess/d.m4a');
  });
});

describe('dictationAudio', () => {
  it('toca a cópia local enquanto existe; sem ela, o arquivo do servidor com o token no cabeçalho', () => {
    expect(dictationAudio(server, 'sess', voice().audio, '/up/sess/d.m4a')).toEqual({ uri: 'file:///doc/draft-attachments/1-1.m4a', name: 'd.m4a' });
    expect(dictationAudio(server, 'sess', null, '/up/sess/x.m4a')).toEqual({
      uri: 'https://a.test/api/sessions/sess/uploads/x.m4a', headers: { Authorization: 'Bearer t' }, name: 'x.m4a',
    });
    expect(dictationAudio(server, 'sess', null, undefined)).toBeNull();
  });
});
