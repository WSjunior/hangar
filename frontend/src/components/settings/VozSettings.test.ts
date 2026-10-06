// @vitest-environment happy-dom
import { describe, it, expect, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import { listarVozesTts } from '@hangar/core';
import VozSettings from './VozSettings.svelte';
import * as m from '../../paraglide/messages';
import type { ConfigServidorStore } from '../../lib/serverConfig.svelte';

vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()), listarVozesTts: vi.fn(async () => []), saldoTts: vi.fn(async () => ({ usados: 0, limite: 0 })), getConfig: vi.fn(async () => ({ campos: {}, somente_leitura: {} })), getTranscriptionProvidersStatus: vi.fn(async () => ({ providers: [] })) }));
vi.mock('../../lib/ttsPlayer.svelte', () => ({ ttsPlayer: { tocando: false, parar: vi.fn() } }));
vi.mock('../../lib/ouvir', () => ({ ouvirAmostra: vi.fn() }));

function montar(campos: Record<string, unknown>, extra: { salvarBloqueado?: boolean } = {}) {
  const alvo = document.createElement('div');
  document.body.appendChild(alvo);
  const store = {
    get alvo() { return null; }, get salvarBloqueado() { return extra.salvarBloqueado ?? false; },
    valorBruto: (chave: string) => (campos[chave] as { valor?: unknown } | undefined)?.valor,
    get campos() { return campos; }, get leitura() { return {}; },
    get carregando() { return false; }, get salvando() { return false; },
    get erro() { return ''; }, get salvo() { return false; }, get temMudanca() { return extra.salvarBloqueado ?? false; },
    valorAtual: (chave: string) => (campos[chave] as { valor?: string } | undefined)?.valor ?? '',
    rascunhoDe: () => '', setRascunho: vi.fn(),
    removerRascunho: vi.fn(), remocaoPendente: () => false, desfazerRascunho: vi.fn(),
    carregar: vi.fn(), salvar: vi.fn(), invalidar: vi.fn(),
  } as unknown as ConfigServidorStore;
  const app = mount(VozSettings, { target: alvo, props: { store } });
  return { alvo, app };
}

describe('VozSettings', () => {
  it('não repete o título da tela e não expõe Groq na superfície principal', () => {
    const { alvo, app } = montar({ groq_api_key: { definido: false, origem: 'env' } });
    expect(alvo.querySelector('h2')).toBeNull();
    expect(alvo.textContent).not.toContain('Groq');
    unmount(app);
  });

  it('oferece endpoint e modelo próprios para outro serviço de transcrição', async () => {
    const { alvo, app } = montar({ groq_api_key: { definido: true, origem: 'app' } });
    alvo.querySelector<HTMLDetailsElement>('.transcription-provider')!.open = true;
    alvo.querySelector<HTMLDetailsElement>('.transcription-provider')!.dispatchEvent(new Event('toggle'));
    await tick();
    expect(alvo.querySelector('#cfg-transcription_base_url')).not.toBeNull();
    expect(alvo.querySelector('#cfg-transcription_model')).not.toBeNull();
    unmount(app);
  });

  it('sem chave de transcrição, avisa que ditar está desligado', () => {
    const { alvo, app } = montar({ groq_api_key: { definido: false } });
    expect(alvo.textContent).toContain(m.voz_transcrever_sem_chave());
    unmount(app);
  });

  it('o avançado do LLM nasce fechado', () => {
    const { alvo, app } = montar({ groq_api_key: { definido: true } });
    expect(alvo.textContent).not.toContain(m.config_server_endpoint_llm());
    expect(alvo.textContent).toContain(m.voz_usar_outro_servico());
    unmount(app);
  });

  it('Mãos-livres fica SEM etiqueta no meio das linhas de servidor — é a ausência que diz "só neste aparelho"', () => {
    const { alvo, app } = montar({ groq_api_key: { definido: true } });
    const maosLivres = alvo.querySelector('.linha-maos-livres')!;
    expect(maosLivres.textContent).toContain(m.config_ditado_titulo());
    expect(maosLivres.querySelector('.escopo')).toBeNull();
    expect(maosLivres.textContent).not.toContain(m.config_escopo_servidor());
    // E as vizinhas, que gravam na máquina, carregam a etiqueta.
    const estilo = alvo.querySelector('.estilo')!;
    expect(estilo.textContent).toContain(m.config_escopo_servidor());
    expect(alvo.textContent).toContain(m.config_server_vocabulario());
    expect(alvo.querySelectorAll('.escopo').length).toBeGreaterThan(1);
    unmount(app);
  });

  it('o seletor de voz tem rótulo visível e a etiqueta de quem grava no servidor', async () => {
    vi.mocked(listarVozesTts).mockResolvedValue([{ id: 'v1', nome: 'Dora' }] as never);
    const { alvo, app } = montar({ elevenlabs_api_key: { definido: true } });
    // A lista de vozes só é buscada a pedido (abrir a tela não pode sair pra rede).
    await tick();
    [...alvo.querySelectorAll('button')].find((b) => b.textContent?.includes(m.config_server_carregar_vozes()))!.click();
    await tick(); await Promise.resolve(); await Promise.resolve(); await tick();
    const bloco = alvo.querySelector('.rot-voz')!;
    expect(bloco.textContent).toContain(m.config_server_voz());
    expect(bloco.textContent).toContain(m.config_escopo_servidor());
    expect(alvo.querySelector('.campo-select')).not.toBeNull();   // o rótulo é DO seletor, que está montado
    unmount(app);
  });

  it('mostra qual voz está salva antes de buscar a lista externa', async () => {
    vi.mocked(listarVozesTts).mockClear();
    const { alvo, app } = montar({
      elevenlabs_api_key: { definido: true, origem: 'app' },
      elevenlabs_voice_id: { valor: 'voz-abc123', origem: 'app' },
    });
    await tick();
    expect(alvo.textContent).toContain('voz-abc123');
    expect(listarVozesTts).not.toHaveBeenCalled();
    unmount(app);
  });

  it('sem chave de voz, a leitura em voz alta aparece desligada, não some', () => {
    const { alvo, app } = montar({ elevenlabs_api_key: { definido: false } });
    expect(alvo.textContent).toContain(m.voz_ler_sem_chave());
    unmount(app);
  });

  it('lista em uso: avisa e abre a seção dos serviços', async () => {
    const { alvo, app } = montar({
      groq_api_key: { definido: true },
      transcription_providers: { valor: [{ id: 'a', kind: 'elevenlabs', name: '', base_url: '', api_key: 'xi_••••', model: '' }], definido: true },
    });
    await tick();
    expect(alvo.textContent).toContain(m.voz_servicos_em_uso());
    expect(alvo.querySelector<HTMLDetailsElement>('.transcription-services')!.open).toBe(true);
    unmount(app);
  });

  it('serviço sem chave: Salvar desligado com o motivo à vista', async () => {
    const { alvo, app } = montar({ groq_api_key: { definido: true } }, { salvarBloqueado: true });
    await tick();
    expect(alvo.textContent).toContain(m.native_server_save_blocked_provider_key());
    const salvar = [...alvo.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === m.ctx_salvar());
    expect(salvar!.disabled).toBe(true);
    unmount(app);
  });
});
