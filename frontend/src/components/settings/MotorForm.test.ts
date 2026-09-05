// @vitest-environment happy-dom
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import MotorForm from './MotorForm.svelte';
import * as apiLib from '../../lib/api';
import * as credLib from '../../lib/credenciais';
import * as m from '../../paraglide/messages';
import type { Motor } from '../../lib/api';

vi.mock('../../lib/api', () => ({
  putEngine: vi.fn(async () => ({ motores: {} })),
  putEngineForServer: vi.fn(async () => ({ motores: {} })),
  engineModelos: vi.fn(async () => ({ modelos: [] })),
  engineModelosForServer: vi.fn(async () => ({ modelos: [] })),
}));
vi.mock('../../lib/credenciais', async (importOriginal) => {
  const real = await importOriginal<typeof import('../../lib/credenciais')>();
  return { ...real, sincronizarNosAgentes: vi.fn(async () => ({ resultado: { pi: { ok: true, motivo: '' } } })) };
});

const apiMock = vi.mocked(apiLib);
const credMock = vi.mocked(credLib);

const KIMI: Motor = {
  label: 'Kimi', base_url: 'https://api.kimi.com/coding', model: 'kimi-k3',
  context_window: 256000, vision: false, api_key: 'sk-k••••4f2a', api_key_definida: true,
  prompt_caching: false,
};

function montar(motor: Motor = KIMI) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const onSalvo = vi.fn();
  const onFechar = vi.fn();
  const comp = mount(MotorForm, { target: el, props: { apiTarget: null, nome: 'kimi', motor, onSalvo, onFechar } });
  return { el, comp: comp as never, onSalvo, onFechar };
}
const campo = (el: HTMLElement, name: string) => el.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
const botao = (el: HTMLElement, rotulo: string) =>
  [...el.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === rotulo)!;

const espera = async () => { for (let i = 0; i < 5; i++) await tick(); };

beforeEach(() => vi.clearAllMocks());

describe('MotorForm', () => {
  it('nasce com o motor lido: modelo, janela, chave definida e campo de chave VAZIO', () => {
    const t = montar();
    expect(campo(t.el, 'model').value).toBe('kimi-k3');
    expect(campo(t.el, 'context_window').value).toBe('256000');
    expect(t.el.textContent).toContain(m.config_motores_chave_definida());
    expect(campo(t.el, 'api_key').value).toBe('');
    expect(campo(t.el, 'api_key').placeholder).toBe(m.config_motores_colar_nova());
    unmount(t.comp);
  });

  it('o avançado nasce fechado', () => {
    const t = montar();
    const det = t.el.querySelector<HTMLDetailsElement>('details');
    expect(det).not.toBeNull();
    expect(det!.open).toBe(false);
    unmount(t.comp);
  });

  it('salvar manda o registro COMPLETO (avançado incluso, vision preservada), chama onSalvo, sincroniza e NÃO fecha', async () => {
    apiMock.putEngine.mockResolvedValueOnce({ motores: { kimi: { ...KIMI, model: 'kimi-k3' } } });
    const t = montar();
    botao(t.el, m.ctx_salvar()).click();
    await tick(); await tick(); await tick();
    expect(apiMock.putEngine).toHaveBeenCalledTimes(1);
    const [nome, corpo] = apiMock.putEngine.mock.calls[0];
    expect(nome).toBe('kimi');
    expect(corpo).toMatchObject({
      label: 'Kimi', base_url: 'https://api.kimi.com/coding', model: 'kimi-k3', context_window: 256000,
      vision: false, prompt_caching: false, adaptive_thinking: true, bundled_skills: false,
    });
    expect(corpo).not.toHaveProperty('api_key');   // chave vazia = mantém a atual
    // 2º argumento: a máquina que RESPONDEU o PUT, capturada antes do await — é o que deixa o pai
    // escrever sob a chave certa mesmo depois de a tela ter trocado de alvo.
    expect(t.onSalvo).toHaveBeenCalledWith({ kimi: expect.objectContaining({ model: 'kimi-k3' }) }, null);
    expect(credMock.sincronizarNosAgentes).toHaveBeenCalledWith(null, 'chave:kimi');
    await tick();
    // O resultado da sincronização fica NA TELA: o bloco não fechou e o botão virou Fechar.
    expect(t.el.textContent).toContain(m.novacred_sync_titulo());
    expect(t.onFechar).not.toHaveBeenCalled();
    expect(botao(t.el, m.sessao_fechar())).toBeDefined();
    unmount(t.comp);
  });

  it('com alvo explícito, salva e sincroniza NAQUELE servidor', async () => {
    const alvo = { id: 'srv-b', label: 'B', baseUrl: 'http://b', token: 't' };
    const el = document.createElement('div');
    document.body.appendChild(el);
    const comp = mount(MotorForm, { target: el, props: { apiTarget: alvo, nome: 'kimi', motor: KIMI, onSalvo: vi.fn(), onFechar: vi.fn() } });
    botao(el, m.ctx_salvar()).click();
    await tick(); await tick(); await tick();
    expect(apiMock.putEngineForServer).toHaveBeenCalledWith(alvo, 'kimi', expect.any(Object));
    expect(apiMock.putEngine).not.toHaveBeenCalled();
    expect(credMock.sincronizarNosAgentes).toHaveBeenCalledWith(alvo, 'chave:kimi');
    unmount(comp);
  });

  it('endereço editado sem chave nova avisa que o Testar usaria o endereço antigo', async () => {
    const t = montar();
    const endereco = campo(t.el, 'base_url');
    endereco.value = 'https://outro.exemplo.com';
    endereco.dispatchEvent(new Event('input', { bubbles: true }));
    await tick();
    expect(t.el.textContent).toContain(m.config_motores_endereco_mudou());
    unmount(t.comp);
  });

  it('motor SEM chave salvo sem chave nova continua sem chave definida', async () => {
    const semChave: Motor = { ...KIMI, api_key: '', api_key_definida: false };
    apiMock.putEngine.mockResolvedValueOnce({ motores: { kimi: semChave } });
    const t = montar(semChave);
    botao(t.el, m.ctx_salvar()).click();
    await tick(); await tick(); await tick();
    expect(apiMock.putEngine.mock.calls[0][1]).not.toHaveProperty('api_key');
    expect(t.el.textContent).not.toContain(m.config_motores_chave_definida());
    expect(campo(t.el, 'api_key').placeholder).toBe(m.config_motores_colar());
    unmount(t.comp);
  });

  it('salvar que falha mostra o erro, não chama onSalvo e não sincroniza', async () => {
    apiMock.putEngine.mockRejectedValueOnce(new Error('502 upstream'));
    const t = montar();
    botao(t.el, m.ctx_salvar()).click();
    await tick(); await tick(); await tick();
    expect(t.el.textContent).toContain('502 upstream');
    expect(t.onSalvo).not.toHaveBeenCalled();
    expect(credMock.sincronizarNosAgentes).not.toHaveBeenCalled();
    // O botão volta a funcionar: `salvando` preso em true deixaria a pessoa sem poder tentar de novo.
    expect(botao(t.el, m.ctx_salvar()).disabled).toBe(false);
    unmount(t.comp);
  });

  it('sincronização que falha NÃO desfaz o motor salvo — o erro aparece e o salvar continua valendo', async () => {
    credMock.sincronizarNosAgentes.mockRejectedValueOnce(new Error('pi fora do ar'));
    const t = montar();
    botao(t.el, m.ctx_salvar()).click();
    await tick(); await tick(); await tick(); await tick();
    expect(t.onSalvo).toHaveBeenCalledTimes(1);
    expect(t.el.textContent).toContain('pi fora do ar');
    expect(t.el.textContent).not.toContain(m.config_motores_erro_salvar());
    expect(botao(t.el, m.sessao_fechar())).toBeDefined();
    unmount(t.comp);
  });

  it('Testar traz os modelos do provedor e o modelo sem janela LIMPA o campo da janela', async () => {
    apiMock.engineModelos.mockResolvedValueOnce({
      modelos: [{ id: 'kimi-k3', context_length: null, vision: null }],
    });
    const t = montar();
    botao(t.el, m.config_motores_testar()).click();
    await tick(); await tick(); await tick();
    expect(apiMock.engineModelos).toHaveBeenCalledWith({ nome: 'kimi' });
    // 256000 veio do motor; o modelo escolhido não declara janela, então o número antigo sai —
    // manter estouraria a janela real do modelo novo.
    expect(campo(t.el, 'context_window').value).toBe('');
    unmount(t.comp);
  });

  it('salvar fica desabilitado sem modelo, e Testar sem endereço', async () => {
    const t = montar({ ...KIMI, model: '', base_url: '' });
    expect(botao(t.el, m.ctx_salvar()).disabled).toBe(true);
    expect(botao(t.el, m.config_motores_testar()).disabled).toBe(true);
    unmount(t.comp);
  });

  // O bloco fica aberto depois de salvar, então salvar duas vezes na mesma instância é o uso normal:
  // o que está na tela tem que ser o resultado da ÚLTIMA gravação.
  it('sync que falha no 2º salvar não deixa o "ok" da 1ª sincronização na tela', async () => {
    const t = montar();
    botao(t.el, m.ctx_salvar()).click();
    await espera();
    expect(t.el.textContent).toContain(m.novacred_sync_ok());

    credMock.sincronizarNosAgentes.mockRejectedValueOnce(new Error('pi fora do ar'));
    botao(t.el, m.ctx_salvar()).click();
    await espera();
    expect(t.el.textContent).toContain('pi fora do ar');
    expect(t.el.textContent).not.toContain(m.novacred_sync_ok());
    unmount(t.comp);
  });

  it('PUT que falha no 2º salvar não deixa o resultado da 1ª sincronização na tela', async () => {
    const t = montar();
    botao(t.el, m.ctx_salvar()).click();
    await espera();
    expect(t.el.textContent).toContain(m.novacred_sync_titulo());

    apiMock.putEngine.mockRejectedValueOnce(new Error('502 upstream'));
    botao(t.el, m.ctx_salvar()).click();
    await espera();
    expect(t.el.textContent).toContain('502 upstream');
    expect(t.el.textContent).not.toContain(m.novacred_sync_titulo());
    unmount(t.comp);
  });

  // Campo AUSENTE do corpo herda o valor do disco (api.put_engine, api.py:3881-3887): omitir o
  // opcional vazio devolvia HTTP 200 com o valor antigo de volta, calado.
  it('limpar o modelo dos subagentes manda subagent_model: "" — omitir faria o backend herdar o antigo', async () => {
    const t = montar({ ...KIMI, subagent_model: 'k2' });
    const sub = campo(t.el, 'subagent_model');
    expect(sub.value).toBe('k2');
    sub.value = '';
    sub.dispatchEvent(new Event('input', { bubbles: true }));
    const janela = campo(t.el, 'context_window');
    janela.value = '';
    janela.dispatchEvent(new Event('input', { bubbles: true }));
    await espera();

    botao(t.el, m.ctx_salvar()).click();
    await espera();
    const corpo = apiMock.putEngine.mock.calls[0][1];
    expect(corpo).toHaveProperty('subagent_model', '');
    // Numérico NÃO tem valor de limpeza: `_normalizar` recusa `''` ("esperado número") e `0`
    // ("deve ser maior que zero") — engines.py:170-179. Vazio segue fora do corpo (= herda o
    // 256000 do disco). Isto trava o corpo no que o backend aceita hoje; apagar a janela pede
    // mudança no backend.
    expect(corpo).not.toHaveProperty('context_window');
    unmount(t.comp);
  });

  // Mesmo caminho do caso acima, no uso comum: abre o motor, não toca em nada, salva. É o que
  // prova que mandar o campo vazio sempre não estraga quem nunca preencheu opcional nenhum.
  it('motor sem opcionais salvo sem tocar em nada manda subagent_model: "" e nenhum numérico', async () => {
    const t = montar({ ...KIMI, subagent_model: undefined, context_window: undefined });
    botao(t.el, m.ctx_salvar()).click();
    await espera();
    const corpo = apiMock.putEngine.mock.calls[0][1];
    expect(corpo).toHaveProperty('subagent_model', '');
    expect(corpo).not.toHaveProperty('context_window');
    expect(corpo).not.toHaveProperty('auto_compact_window');
    expect(corpo).not.toHaveProperty('max_output_tokens');
    unmount(t.comp);
  });

  it('cancelar chama onFechar sem gravar', () => {
    const t = montar();
    botao(t.el, m.comum_cancelar()).click();
    expect(t.onFechar).toHaveBeenCalledTimes(1);
    expect(apiMock.putEngine).not.toHaveBeenCalled();
    unmount(t.comp);
  });
});
