// @vitest-environment happy-dom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Composer from './Composer.svelte';
import * as m from '../paraglide/messages';
import { configureApi, transcribeUploaded, uploadFile } from '@hangar/core';
import { dictations, dictationBarKey, draftStorageKey } from '../lib/dictationStore.svelte';

vi.mock('@hangar/core', async (orig) => ({
  ...(await orig<typeof import('@hangar/core')>()),
  getPermissionModes: vi.fn(async () => ({ current: 'plan', modes: ['plan'] })),
  getCommands: vi.fn(async () => []),
  getModelOptions: vi.fn(async () => []),
  uploadFile: vi.fn(),
  transcribeUploaded: vi.fn(),
}));
vi.mock('../lib/sessionsStore.svelte', () => ({
  sessionsStore: { epoca: () => 0, retain: vi.fn(), release: vi.fn(), sessionsForServer: () => [] },
}));

const flush = async () => { await tick(); await new Promise((r) => setTimeout(r, 0)); await tick(); };
const audio = () => new File(['a'], 'g.webm', { type: 'audio/webm' });
const BARRA = dictationBarKey('', 's');
const iniciar = () => dictations.start({ serverId: '', name: 's', jsonl: 'j1', server: undefined, file: audio(), opts: { ditado: true, autoEnvio: false } });

function colarAudio(target: Element, f: File) {
  const ev = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(ev, 'clipboardData', { value: { items: [{ kind: 'file', type: f.type, getAsFile: () => f }] } });
  target.querySelector('textarea')!.dispatchEvent(ev);
}

function montar(sessionJsonl: string | null = 'j1') {
  const target = document.createElement('div');
  document.body.appendChild(target);
  const props = $state({
    sessionName: 's', sessionState: 'idle' as const, status: null, inputText: '', sessionJsonl,
    onSend: vi.fn(), onCommand: vi.fn(), onInterrupt: vi.fn(), onOpenGit: vi.fn(), onOpenPreview: vi.fn(),
  });
  const comp = mount(Composer, { target, props });
  return { target, comp, props };
}

beforeEach(() => {
  dictations._resetForTests();
  localStorage.clear();
  vi.clearAllMocks();
  configureApi({ getBaseUrl: () => 'http://h', getToken: () => 't', onUnauthorized: () => {}, origin: null,
    createEventSource: () => { throw new Error('sem SSE no teste'); } });
  // Config do estilo do ditado e afins: nenhuma rede de verdade no teste.
  vi.spyOn(globalThis, 'fetch').mockImplementation(async () => new Response('{}', { status: 200 }));
  vi.mocked(uploadFile).mockResolvedValue({ path: '/up/p/s/g.webm' });
});

describe('Composer e o ditado da sessão', () => {
  it('remontado durante a transcrição mostra "transcrevendo" e trava o mic', async () => {
    vi.mocked(transcribeUploaded).mockReturnValue(new Promise(() => {}));
    iniciar();
    const { target, comp } = montar();
    await flush();
    expect(target.querySelector<HTMLButtonElement>('.mic-btn')!.disabled).toBe(true);
    expect(target.textContent).toContain(m.board_transcrevendo());
    unmount(comp);
  });

  it('resultado com a conversa aberta entra no campo e a barra guarda o transcript', async () => {
    let resolver!: (v: { path: string; text: string; raw: string }) => void;
    vi.mocked(transcribeUploaded).mockReturnValue(new Promise((r) => { resolver = r; }));
    const { target, comp } = montar('j1');
    await flush();
    iniciar();
    await flush();
    resolver({ path: '/up/p/s/g.webm', text: 'olá', raw: 'ola' });
    await flush();
    expect(target.querySelector('textarea')!.value).toBe('olá');
    expect(target.querySelector('.ditado-bar')).not.toBeNull();
    expect(JSON.parse(localStorage.getItem(BARRA)!)).toMatchObject({ arquivo: 'g.webm', jsonl: 'j1' });
    unmount(comp);
  });

  it('com rascunho guardado ainda não conferido, espera o transcript e então insere', async () => {
    localStorage.setItem(draftStorageKey('', 's'), JSON.stringify({ text: 'meu rascunho', jsonl: 'j1' }));
    vi.mocked(transcribeUploaded).mockResolvedValue({ path: '/up/p/s/g.webm', text: 'olá', raw: 'ola' });
    const { target, comp, props } = montar(null);
    await flush();
    iniciar();
    await flush();
    expect(target.querySelector('textarea')!.value).toBe('');
    expect(dictations.get('', 's')?.status).toBe('ready');
    props.sessionJsonl = 'j1';
    await flush();
    expect(target.querySelector('textarea')!.value).toBe('olá');
    unmount(comp);
  });

  it('barra guardada de outro transcript não volta', async () => {
    localStorage.setItem(BARRA, JSON.stringify({ arquivo: 'g.webm', raw: 'x', before: '', after: '', jsonl: 'velho' }));
    const { target, comp } = montar('novo');
    await flush();
    expect(target.querySelector('.ditado-bar')).toBeNull();
    expect(localStorage.getItem(BARRA)).toBeNull();
    unmount(comp);
  });

  it('barra na chave antiga é movida e volta', async () => {
    localStorage.setItem('cp-ditado:s', JSON.stringify({ arquivo: 'g.webm', raw: 'x', before: '', after: '', jsonl: 'j1' }));
    const { target, comp } = montar('j1');
    await flush();
    expect(target.querySelector('.ditado-bar')).not.toBeNull();
    expect(localStorage.getItem('cp-ditado:s')).toBeNull();
    unmount(comp);
  });

  it('aviso do resultado (reserva) sai em tom de aviso, não de erro', async () => {
    vi.mocked(transcribeUploaded).mockResolvedValue({ path: '/up/p/s/g.webm', text: 'olá', raw: 'ola', aviso: 'Transcrito pelo Groq: sem cota' });
    const { target, comp } = montar('j1');
    await flush();
    iniciar();
    await flush();
    const aviso = target.querySelector('.send-error')!;
    expect(aviso.textContent).toContain('Transcrito pelo Groq: sem cota');
    expect(aviso.classList.contains('send-error--aviso')).toBe(true);
    expect(aviso.getAttribute('role')).toBe('status');
    unmount(comp);
  });

  it('áudio que o servidor não entrega mais: a barra diz isso', async () => {
    localStorage.setItem(BARRA, JSON.stringify({ arquivo: 'g.webm', raw: 'x', before: '', after: '', jsonl: 'j1' }));
    const { target, comp } = montar('j1');
    await flush();
    target.querySelector('.ditado-audio')!.dispatchEvent(new Event('error'));
    await flush();
    expect(target.textContent).toContain(m.composer_ditado_audio_indisponivel());
    expect(target.querySelector('.send-error--aviso')).toBeNull();
    unmount(comp);
  });

  it('áudio recusado com outra transcrição no ar vai para os anexos e avisa', async () => {
    vi.mocked(transcribeUploaded).mockReturnValue(new Promise(() => {}));
    iniciar();
    const { target, comp } = montar();
    await flush();
    const f = new File(['b'], 'nota.m4a', { type: 'audio/mp4' });
    colarAudio(target, f);
    await flush();
    expect(uploadFile).toHaveBeenLastCalledWith('s', f, undefined, undefined, { audioOnly: true });
    expect(target.textContent).toContain(m.composer_aguarde_transcricao());
    unmount(comp);
  });

  it('gravação só no aparelho: o áudio novo é guardado e o aviso diz o motivo; falha ao guardar aparece', async () => {
    vi.mocked(uploadFile).mockRejectedValueOnce(new Error('rede'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    iniciar();
    const { target, comp } = montar();
    await flush();
    const f = new File(['b'], 'nota.m4a', { type: 'audio/mp4' });
    colarAudio(target, f);
    await flush();
    expect(uploadFile).toHaveBeenLastCalledWith('s', f, undefined, undefined, { audioOnly: true });
    expect(target.textContent).toContain(m.composer_ditado_so_no_aparelho());
    vi.mocked(uploadFile).mockRejectedValueOnce(new Error('sem espaço'));
    colarAudio(target, f);
    await flush();
    expect(target.textContent).toContain(m.composer_ditado_nao_guardado({ erro: 'sem espaço' }));
    unmount(comp);
  });

  it('falha mostra o erro com "Transcrever de novo"', async () => {
    vi.mocked(transcribeUploaded).mockRejectedValue(Object.assign(new Error('502: fora'), { status: 502 }));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    iniciar();
    const { target, comp } = montar();
    await flush();
    expect(target.textContent).toContain('502: fora');
    expect(target.textContent).toContain(m.composer_transcrever_de_novo());
    unmount(comp);
  });
});
