// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { UploadFile } from '@hangar/core';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const h = vi.hoisted(() => ({ list: vi.fn(), dictate: vi.fn(), back: vi.fn(), push: vi.fn(), toast: vi.fn() }));
vi.mock('expo-router', () => ({
  useLocalSearchParams: () => ({ server: 's1', name: 'sess' }),
  useRouter: () => ({ push: h.push, back: h.back }),
}));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  listUploads: h.list,
  uploadUrlNative: (name: string, filename: string) => `https://a.test/${name}/${filename}`,
  fileUrlNative: (name: string, filename: string) => `https://a.test/file/${name}/${filename}`,
  fileAuthHeader: () => ({ Authorization: 'Bearer t' }),
}));
vi.mock('../../stores/servers', () => {
  const state = { ready: true, servers: [{ id: 's1' }], ensureActive: () => true };
  return { useServers: Object.assign((select: (s: typeof state) => unknown) => select(state), { getState: () => state }) };
});
vi.mock('../../stores/sessions', () => {
  const state = { rows: [{ serverId: 's1', name: 'sess', jsonl: '/t/a.jsonl' }], byServerRecord: {} };
  return { useSessions: Object.assign((select: (s: typeof state) => unknown) => select(state), { getState: () => state }) };
});
vi.mock('../ditado/ditadoEstiloStore', () => ({ useDitadoEstiloStore: { getState: () => ({ pronto: true, valor: 'prosa' }) } }));
vi.mock('../ditado/dictationRun', () => ({
  dictateUpload: h.dictate,
  DictationError: class extends Error {
    constructor(message: string, readonly lost: boolean, readonly storageIssue: string) { super(message); }
  },
}));
vi.mock('../../chat/AudioChip', () => ({ AudioChip: ({ name }: { name: string }) => createElement('span', null, `audio:${name}`) }));
vi.mock('./AttachmentCard', () => ({
  AttachmentCard: ({ file, onPress }: { file: UploadFile; onPress: () => void }) =>
    createElement('button', { 'aria-label': file.filename, onClick: onPress }, file.filename),
}));
vi.mock('./Lightbox', () => ({ Lightbox: () => null }));
vi.mock('./DocumentViewer', () => ({
  DocumentViewer: ({ doc }: { doc: { kind: string; name: string } | null }) => (doc ? createElement('div', { 'data-doc': doc.kind }, doc.name) : null),
}));
vi.mock('../../ui/Toast', () => ({ toast: { erro: h.toast, ok: () => {} } }));
vi.mock('../../theme/superficie', () => ({ superficie: () => '#eee' }));
vi.mock('../../paraglide/messages', () => Object.fromEntries(
  ('comum_carregando anexos_erro_listar anexos_nenhum lista_tentar_novamente ctx_anexos composer_transcrever_de_novo '
    + 'arq_sessao_encerrada sessao_expirada anexos_fechar_visualizacao arquivo_carregar_erro chat_servidor_removido composer_falha_transcricao')
    .split(' ').map((k) => [k, () => k]),
));

import AttachmentsScreen from '../../../app/s/[server]/[name]/attachments';

const file = (filename: string): UploadFile => ({ filename, size: 10, mtime: 0, expires_in_days: null });
async function render() {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(createElement(AttachmentsScreen)));
  return { container, root };
}
const byLabel = (c: HTMLElement, label: string) => [...c.querySelectorAll('button')]
  .find((el) => el.textContent === label || el.getAttribute('aria-label') === label) as HTMLButtonElement | undefined;

describe('galeria de anexos', () => {
  beforeEach(() => { h.list.mockReset(); h.dictate.mockReset(); h.back.mockReset(); h.toast.mockReset(); });

  it('carregando, vazio, erro com tentar de novo e lista', async () => {
    let finish!: (v: { files: UploadFile[] }) => void;
    h.list.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const first = await render();
    expect(first.container.textContent).toContain('comum_carregando');
    await act(async () => finish({ files: [] }));
    expect(first.container.textContent).toContain('anexos_nenhum');
    act(() => first.root.unmount());

    h.list.mockRejectedValueOnce(new Error('caiu')).mockResolvedValueOnce({ files: [file('foto.png')] });
    const second = await render();
    expect(second.container.textContent).toContain('anexos_erro_listar caiu');
    await act(async () => byLabel(second.container, 'lista_tentar_novamente')!.click());
    expect(byLabel(second.container, 'foto.png')).toBeDefined();
    act(() => second.root.unmount());
  });

  it('áudio toca na lista e "Transcrever de novo" manda à conversa e volta', async () => {
    h.list.mockResolvedValueOnce({ files: [file('ditado-1.m4a'), file('clip.mp4')] });
    h.dictate.mockReturnValueOnce(Promise.resolve());
    const { container, root } = await render();
    expect(container.textContent).toContain('audio:ditado-1.m4a');
    await act(async () => byLabel(container, 'composer_transcrever_de_novo: ditado-1.m4a')!.click());
    expect(h.dictate).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 's1', 'sess', '/t/a.jsonl', 'ditado-1.m4a', 'prosa');
    expect(h.back).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
  });

  it('falha que não ficou guardada na conversa vira toast; a guardada não', async () => {
    const { DictationError } = await import('../ditado/dictationRun');
    h.list.mockResolvedValue({ files: [file('ditado-1.m4a')] });
    h.dictate
      .mockRejectedValueOnce(new DictationError('502: fora', false, ''))
      .mockRejectedValueOnce(new DictationError('502: fora', true, ''))
      .mockRejectedValueOnce(new DictationError('502: fora', false, 'disco cheio'));
    const { container, root } = await render();
    const botao = () => byLabel(container, 'composer_transcrever_de_novo: ditado-1.m4a')!;
    await act(async () => botao().click());
    expect(h.toast).not.toHaveBeenCalled();
    await act(async () => botao().click());
    expect(h.toast).toHaveBeenLastCalledWith('502: fora');
    await act(async () => botao().click());
    expect(h.toast).toHaveBeenLastCalledWith('502: fora (disco cheio)');
    act(() => root.unmount());
  });

  it('vídeo abre no visualizador de documento', async () => {
    h.list.mockResolvedValueOnce({ files: [file('clip.mp4')] });
    const { container, root } = await render();
    await act(async () => byLabel(container, 'clip.mp4')!.click());
    expect(container.querySelector('[data-doc="video"]')?.textContent).toBe('clip.mp4');
    act(() => root.unmount());
  });

  it('ditado da conversa ainda não usado: a recusa aparece e a galeria fica', async () => {
    h.list.mockResolvedValueOnce({ files: [file('ditado-1.m4a')] });
    h.dictate.mockImplementationOnce(() => { throw new Error('composer_aguarde_transcricao'); });
    const { container, root } = await render();
    await act(async () => byLabel(container, 'composer_transcrever_de_novo: ditado-1.m4a')!.click());
    expect(h.toast).toHaveBeenCalledWith('composer_aguarde_transcricao');
    expect(h.back).not.toHaveBeenCalled();
    act(() => root.unmount());
  });
});
