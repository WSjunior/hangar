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
// expo-file-system não carrega no node; a galeria só apaga a cópia local do ditado substituído.
vi.mock('../../chat/draftAttachments', () => ({ removeDraftAttachment: vi.fn() }));

import { applyReadyDictation, readDictation, readDraft, setDictationInFlight, writeDictation, writeDraft } from '../../stores/drafts';
import { dictateUpload, dictationAudio, runDictation } from './dictationRun';
import { removeDraftAttachment } from '../../chat/draftAttachments';

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

describe('dictateUpload', () => {
  beforeEach(() => { memory.clear(); core.transcribe.mockReset(); core.upload.mockReset(); vi.mocked(removeDraftAttachment).mockClear(); });

  it('galeria transcreve pelo nome guardado, sem subir cópia, e o texto entra no rascunho intacto', async () => {
    writeDraft('s1', 'sess', { version: 1, text: 'antes', revision: 2, transcript: '/t/a.jsonl', attachment: null, submission: null });
    core.transcribe.mockResolvedValueOnce({ path: '/up/sess/ditado-9.m4a', text: ' depois ', raw: 'depois cru', aviso: null, estilo_aplicado: 'prosa' });
    await dictateUpload(server, 's1', 'sess', '/t/a.jsonl', 'ditado-9.m4a', 'prosa');
    expect(core.transcribe).toHaveBeenCalledExactlyOnceWith(server, 'sess', 'ditado-9.m4a', { limpar: true, estilo: 'prosa' });
    expect(core.upload).not.toHaveBeenCalled();
    expect(readDictation('s1', 'sess')).toMatchObject({
      status: 'ready', text: 'depois', audio: null, serverPath: '/up/sess/ditado-9.m4a', draftRevision: 2, before: 'antes',
    });
    expect(applyReadyDictation('s1', 'sess', '/t/a.jsonl')?.draft?.text).toBe('antes depois');
    expect(readDraft('s1', 'sess')?.text).toBe('antes depois');
  });

  it('durante o POST o ditado fica pendente para quem montar a conversa', async () => {
    let finish!: (v: unknown) => void;
    core.transcribe.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const run = dictateUpload(server, 's1', 'sess', null, 'a.m4a');
    expect(readDictation('s1', 'sess')?.status).toBe('pending');
    finish({ path: '/up/a.m4a', text: 'ok' });
    await run;
    expect(readDictation('s1', 'sess')?.status).toBe('ready');
  });

  it('recusa na hora quando há ditado pronto ainda não usado', () => {
    writeDictation('s1', 'sess', voice({ status: 'ready', text: 'guardado' }));
    expect(() => dictateUpload(server, 's1', 'sess', null, 'a.m4a')).toThrow();
    expect(core.transcribe).not.toHaveBeenCalled();
  });

  it('recusa enquanto o ditado da conversa (gravado na tela por baixo) está no ar', () => {
    writeDictation('s1', 'sess', voice());
    setDictationInFlight('s1', 'sess', 'v1', true);
    expect(() => dictateUpload(server, 's1', 'sess', null, 'a.m4a')).toThrow();
    expect(readDictation('s1', 'sess')?.id).toBe('v1');
    expect(core.transcribe).not.toHaveBeenCalled();
    setDictationInFlight('s1', 'sess', 'v1', false);
  });

  it('ditado com falha que já subiu é substituído pelo da galeria e a cópia local sai', async () => {
    writeDictation('s1', 'sess', voice({ status: 'failed', issue: '502: groq', serverPath: '/up/sess/d.m4a' }));
    core.transcribe.mockResolvedValueOnce({ path: '/up/sess/b.m4a', text: 'novo' });
    await dictateUpload(server, 's1', 'sess', null, 'b.m4a');
    expect(readDictation('s1', 'sess')).toMatchObject({ status: 'ready', text: 'novo', audio: null });
    expect(removeDraftAttachment).toHaveBeenCalledWith('file:///doc/draft-attachments/1-1.m4a');
  });

  it('recusa quando a gravação com falha só existe no aparelho (não subiu)', () => {
    writeDictation('s1', 'sess', voice({ status: 'failed', issue: 'rede caiu' }));
    expect(() => dictateUpload(server, 's1', 'sess', null, 'b.m4a')).toThrow();
    expect(readDictation('s1', 'sess')).toMatchObject({ id: 'v1', audio: { uri: 'file:///doc/draft-attachments/1-1.m4a' } });
    expect(removeDraftAttachment).not.toHaveBeenCalled();
    expect(core.transcribe).not.toHaveBeenCalled();
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
