// @vitest-environment happy-dom
// A folha de "+ Adicionar" abre num passo ZERO: o que você quer adicionar? Sem ele, o catálogo de
// provedores respondia sozinho uma pergunta que nunca foi feita — e "modelo pro Claude Code", que é
// um dos três usos, não era alcançável por porta nenhuma da interface.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import NovaCredencialSheet from './NovaCredencialSheet.svelte';
import * as m from '../../paraglide/messages';
import * as core from '@hangar/core';

vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()),
  criarConta: vi.fn(async () => ({ path: '/x', label: 'x', active: false })),
  putEngine: vi.fn(async () => ({ motores: {} })),
  putEngineForServer: vi.fn(async () => ({ motores: {} })),
  engineModelos: vi.fn(async () => ({ modelos: [] })),
  engineModelosForServer: vi.fn(async () => ({ modelos: [] })),
  engineCliproxy: vi.fn(async () => ({ found: false, base_url: null, models: [], error: null })),
  engineCliproxyForServer: vi.fn(async () => ({ found: false, base_url: null, models: [], error: null })),
}));
vi.mock('../../lib/credenciais', () => ({
  sincronizarNosAgentes: vi.fn(async () => ({ resultado: { pi: { ok: true, motivo: '' } } })),
  codexLoginIniciar: vi.fn(async () => ({ etapa: 'aguardando', url: 'https://x', user_code: 'ABC' })),
  codexLoginPasso: vi.fn(async () => ({ etapa: 'aguardando', url: 'https://x', user_code: 'ABC' })),
  codexLoginCancelar: vi.fn(async () => ({ ok: true })),
}));

const onFechar = vi.fn();
const onCriada = vi.fn();

function montar() {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const comp = mount(NovaCredencialSheet, {
    target: el,
    props: { apiTarget: null, onFechar, onCriada },
  });
  return { el, comp: comp as never, fim: () => { unmount(comp as never); el.remove(); } };
}

// O botão é achado pelo TEXTO da linha que ele fecha — a folha não tem id por item, e casar por
// posição quebraria a cada linha nova do catálogo.
function conectarDe(rotulo: string): HTMLButtonElement {
  const item = [...document.body.querySelectorAll<HTMLElement>('.nc-item')]
    .find((i) => i.textContent?.includes(rotulo));
  expect(item, `linha "${rotulo}" não está na tela`).toBeTruthy();
  return item!.querySelector<HTMLButtonElement>('.nc-conectar')!;
}

beforeEach(() => vi.clearAllMocks());

describe('NovaCredencialSheet — o passo "o quê"', () => {
  it('abre perguntando o quê, com as três opções e SEM o catálogo de provedores', async () => {
    const t = montar();
    await tick();
    expect(document.body.textContent).toContain(m.contas_nova_escolha());
    expect(document.body.textContent).toContain(m.contas_add_conta());
    expect(document.body.textContent).toContain(m.contas_add_modelo());
    expect(document.body.textContent).toContain(m.contas_add_chave());
    expect(document.body.querySelectorAll('.nc-item').length).toBe(3);
    // O catálogo responderia a pergunta antes de ela ser feita.
    expect(document.body.textContent).not.toContain(m.novacred_kimi_desc());
    expect(document.body.textContent).not.toContain('OpenCode Zen');
    t.fim();
  });

  it('as três opções têm ícone de traço, não as INICIAIS do rótulo', async () => {
    // "MO"/"CH" num quadradinho lê como placeholder que ninguém terminou: iniciais servem pra
    // distinguir MARCA numa lista de provedores, e escolha de caminho não é marca nenhuma.
    const t = montar();
    await tick();
    for (const item of document.body.querySelectorAll<HTMLElement>('.nc-item')) {
      expect(item.querySelector('svg')).not.toBeNull();
      expect(item.textContent).not.toMatch(/\b[A-Z]{2}\b/);
    }
    t.fim();
  });

  it('assinatura oferece Claude e Codex antes do nome', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_conta()).click();
    await tick();
    expect(document.body.textContent).toContain(m.novacred_codex_nome());
    conectarDe(m.novacred_claude_nome()).click();
    await tick();
    expect(document.body.textContent).toContain(m.novacred_nome_conta());
    t.fim();
  });

  it('"Modelo pro Claude Code" mostra o catálogo sem as linhas de login e com o OmniRoute', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_modelo()).click();
    await tick();
    expect(document.body.querySelector('.nc-lista')).not.toBeNull();
    expect(document.body.textContent).toContain('OmniRoute');
    expect(document.body.textContent).toContain('Kimi Code');
    // Entrar numa conta não é cadastrar modelo: as duas linhas de login ficam fora daqui.
    expect(document.body.textContent).not.toContain(m.novacred_claude_desc());
    expect(document.body.textContent).not.toContain(m.novacred_codex_desc());
    t.fim();
  });

  it('"Modelo pro Claude Code" + provedor abre o formulário COMPLETO do motor, já com a URL', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_modelo()).click();
    await tick();
    conectarDe('Kimi Code').click();
    await tick();
    // O formulário curto de chave não serve aqui: sem modelo, janela e avançado, o motor nasce
    // incompleto e a sessão compacta a 200k.
    expect(document.body.querySelector('.mf')).not.toBeNull();
    expect(document.body.textContent).toContain(m.config_motores_avancado());
    expect(document.body.querySelector<HTMLInputElement>('input[name="base_url"]')!.value)
      .toBe('https://api.kimi.com/coding');
    t.fim();
  });

  it('chave preserva catálogo API sem login Codex', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_chave()).click();
    await tick();
    expect(document.body.textContent).not.toContain(m.novacred_codex_nome());
    // E a conta do Claude não é chave de agente nenhum.
    expect(document.body.textContent).not.toContain(m.novacred_claude_desc());
    t.fim();
  });

  it('assinatura ChatGPT abre cadastro sem URL ou segredo de API', async () => {
    localStorage.setItem('cp_servers', JSON.stringify([{ id: 'b', label: 'B', baseUrl: 'https://b.test', token: 'b' }]));
    localStorage.setItem('cp_active', 'b');
    const t = montar(); await tick();
    conectarDe(m.contas_add_conta()).click(); await tick();
    conectarDe(m.novacred_codex_nome()).click(); await tick();
    expect(document.body.textContent).toContain(m.codex_ui_intro());
    expect(document.querySelector('input[type="password"]')).toBeNull();
    expect(document.querySelector('input[type="url"]')).toBeNull();
    expect(document.body.textContent).toContain(m.novacred_nome_conta());
    t.fim(); localStorage.clear();
  });

  it('voltar do catálogo devolve ao passo "o quê" — não fecha a folha', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_chave()).click();
    await tick();
    document.body.querySelector<HTMLButtonElement>('.nc-voltar')!.click();
    await tick();
    expect(document.body.textContent).toContain(m.contas_nova_escolha());
    expect(document.body.querySelectorAll('.nc-item').length).toBe(3);
    expect(onFechar).not.toHaveBeenCalled();
    // No passo "o quê" não há um passo anterior — o ← some.
    expect(document.body.querySelector('.nc-voltar')).toBeNull();
    t.fim();
  });

  it('voltar do formulário devolve ao catálogo, um passo de cada vez', async () => {
    const t = montar();
    await tick();
    conectarDe(m.contas_add_chave()).click();
    await tick();
    conectarDe('Groq').click();
    await tick();
    document.body.querySelector<HTMLButtonElement>('.nc-voltar')!.click();
    await tick();
    expect(document.body.querySelector('.nc-lista')).not.toBeNull();
    expect(document.body.textContent).toContain('Groq');
    expect(onFechar).not.toHaveBeenCalled();
    t.fim();
  });
});

describe('NovaCredencialSheet — CLIProxyAPI detectado pelo servidor', () => {
  const URL_CP = 'http://127.0.0.1:8317';
  const achou = { found: true, base_url: URL_CP, models: [{ id: 'gpt-x', context_length: 400000, vision: true }], error: null };
  // A detecção é uma promessa: um tick só não basta pra resposta aterrissar na tela.
  const assentar = async () => { await new Promise((r) => setTimeout(r, 0)); await tick(); };

  async function abrirCliproxy(caminho: string) {
    const t = montar();
    await tick();
    conectarDe(caminho).click();
    await tick();
    conectarDe('CLIProxyAPI').click();
    await assentar();
    return t;
  }

  it('achou: sem campo de chave, e salvar pede a chave do servidor em vez de mandá-la', async () => {
    vi.mocked(core.engineCliproxy).mockResolvedValueOnce(achou);
    const t = await abrirCliproxy(m.contas_add_chave());
    expect(document.body.textContent).toContain(m.cliproxy_found({ url: URL_CP }));
    expect(document.querySelector('input[type="password"]')).toBeNull();
    document.body.querySelector<HTMLButtonElement>('.nc-btn.primario')!.click();
    await assentar();
    const [id, dados] = vi.mocked(core.putEngine).mock.calls[0];
    expect(id).toBe('cliproxyapi');
    expect(dados).toMatchObject({ base_url: URL_CP, model: 'gpt-x', use_cliproxy_key: true });
    expect(dados).not.toHaveProperty('api_key');
    t.fim();
  });

  it('achou no caminho "modelo": o formulário do motor também some com a chave', async () => {
    vi.mocked(core.engineCliproxy).mockResolvedValueOnce(achou);
    const t = await abrirCliproxy(m.contas_add_modelo());
    expect(document.querySelector('input[name="api_key"]')).toBeNull();
    const nomeEl = document.querySelector<HTMLInputElement>('input[name="nome"]')!;
    nomeEl.value = 'cp';
    nomeEl.dispatchEvent(new Event('input', { bubbles: true }));
    await tick();
    document.body.querySelector<HTMLButtonElement>('.mf .btn.primario')!.click();
    await assentar();
    const [, dados] = vi.mocked(core.putEngine).mock.calls[0];
    expect(dados).toMatchObject({ base_url: URL_CP, model: 'gpt-x', context_window: 400000, use_cliproxy_key: true });
    expect(dados).not.toHaveProperty('api_key');
    t.fim();
  });

  it('não instalado: avisa e cai no formulário manual com chave', async () => {
    const t = await abrirCliproxy(m.contas_add_chave());
    expect(document.body.textContent).toContain(m.cliproxy_missing());
    expect(document.querySelector('input[type="password"]')).not.toBeNull();
    t.fim();
  });

  it('achou mas não respondeu: mostra o erro e deixa digitar a chave, com a URL preenchida', async () => {
    vi.mocked(core.engineCliproxy).mockResolvedValueOnce({ found: true, base_url: URL_CP, models: [], error: 'connection refused' });
    const t = await abrirCliproxy(m.contas_add_chave());
    expect(document.body.textContent).toContain(m.cliproxy_error({ url: URL_CP, erro: 'connection refused' }));
    expect(document.querySelector('input[type="password"]')).not.toBeNull();
    expect(document.querySelector<HTMLInputElement>('input[type="url"]')!.value).toBe(URL_CP);
    t.fim();
  });
});
