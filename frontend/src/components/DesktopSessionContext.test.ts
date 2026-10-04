// @vitest-environment happy-dom
// Follow-up visual: com toggle externo (barra/rail), o DesktopSessionContext NÃO
// pode ter botão duplicado (.ctx-fold) nem aba vertical central quando recolhido — o painel
// simplesmente some. Sem toggle externo (sidebar expandida), a porta acessível do painel continua.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import DesktopSessionContext from './DesktopSessionContext.svelte';
import { ctxPanel, LARGURA_MIN, LARGURA_ABERTO } from '../lib/ctxPanel.svelte';
import { overwriteGetLocale } from '../paraglide/runtime';
import * as m from '../paraglide/messages';
import { listFiles, configureLocale } from '@hangar/core';

// Stubs dos componentes internos pesados (PlanRing/PlanPanel renderizam SVG/estado de plano).
vi.mock('./PlanRing.svelte', () => ({ default: class { $destroy() {} } }));
vi.mock('./PlanPanel.svelte', () => ({ default: class { $destroy() {} } }));

// A aba Arquivos (FilesPanel) fala com a rede no mount — sem o mock o teste montaria o
// componente com fetch de verdade.
vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()),
  getPermissionModes: vi.fn().mockResolvedValue({ current: 'plan', modes: ['plan', 'auto', 'manual', 'acceptEdits'] }),
  setPermissionMode: vi.fn().mockResolvedValue({ mode: 'plan', current: 'plan' }),
  isTimeoutError: vi.fn(() => false),
  isAbortError: vi.fn(() => false),
  listFiles: vi.fn().mockResolvedValue({ entries: [], truncated: false }),
  getChangedFiles: vi.fn().mockResolvedValue({
    files: [
      { path: 'src/a.ts', code: ' M', staged: false, added: 5, removed: 1 },
      { path: 'src/components/Grande.svelte', code: ' M', staged: false, added: 90, removed: 4 },
      { path: 'docs/meio.md', code: ' M', staged: false, added: 20, removed: 0 },
      { path: 'bin/blob.png', code: ' M', staged: false, added: null, removed: null },
    ],
    sequencer: null,
  }),
  readFile: vi.fn(),
  searchFiles: vi.fn(),
  pathDiff: vi.fn(),
}));

function montar(toggleExterno: boolean) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const comp = mount(DesktopSessionContext, {
    target: el,
    props: {
      state: 'idle',
      sessionName: 'sess-1',
      serverId: 'srv-test',
      toggleExterno,
    },
  });
  return { el, comp: comp as never };
}

beforeEach(() => {
  overwriteGetLocale(() => 'pt');   // textos dos painéis são mensagens agora
  ctxPanel.recolhido = false;
  ctxPanel.aba = 'contexto';        // a aba vive no modulo — reset entre testes
  ctxPanel.largura = LARGURA_ABERTO; // idem: largura vive no modulo
  document.body.innerHTML = '';
});

describe('DesktopSessionContext — toggle na barra (follow-up visual)', () => {
  it('toggleExterno: NENHUM .ctx-fold (nem no aberto) — sem botão duplicado', async () => {
    const t = montar(true);
    await tick();
    expect(document.querySelector('.ctx-fold')).toBeNull();
    unmount(t.comp);
  });

  it('toggleExterno + recolhido: painel some (sem aba vertical central)', async () => {
    const t = montar(true);
    await tick();
    ctxPanel.recolhido = true;
    await tick();
    const aside = document.querySelector<HTMLElement>('.session-context');
    expect(aside?.classList.contains('recolhido')).toBe(true);
    expect(aside?.classList.contains('toggle-externo')).toBe(true);
    // nenhuma aba vertical: não existe .ctx-fold (o display:none da regra recolhido+toggle-externo
    // é validado no browser — happy-dom não injeta o CSS escopado)
    expect(document.querySelector('.ctx-fold')).toBeNull();
    unmount(t.comp);
  });

  it('sem toggleExterno (sidebar expandida): porta acessível no TOPO preservada — sem aba flutuante isolada', async () => {
    const t = montar(false);
    await tick();
    const fold = document.querySelector<HTMLButtonElement>('.ctx-fold');
    expect(fold).not.toBeNull();
    expect(fold?.getAttribute('aria-label')).toBe('Recolher contexto');
    ctxPanel.recolhido = true;
    await tick();
    // A porta continua (botão do header, não aba 26×64 top:50%): o CSS da aba isolada morreu —
    // o ctx-fold fica na posição do header (estático no topo), não flutuando no meio da borda.
    expect(document.querySelector('.ctx-fold')).not.toBeNull();
    const aside = document.querySelector<HTMLElement>('.session-context')!;
    expect(aside.classList.contains('recolhido')).toBe(true);
    expect(aside.classList.contains('toggle-externo')).toBe(false);
    unmount(t.comp);
  });

  it('a barra de abas tem Contexto e Arquivos por padrão — Navegador só aparece quando a sessão TEM navegador aberto', async () => {
    const t = montar(false);
    await tick();
    const abas = [...document.querySelectorAll('.aba')];
    expect(abas.map((a) => a.textContent?.trim())).toEqual(['Contexto', 'Arquivos']);
    expect(abas[0].getAttribute('aria-selected')).toBe('true');
    expect(abas[1].getAttribute('aria-selected')).toBe('false');
    unmount(t.comp);
  });

  it('com navegador aberto na sessão, a aba Navegador nasce na barra', async () => {
    const { marcarNavAberto, fecharNav } = await import('../lib/navegadorPanel.svelte');
    marcarNavAberto('srv-test::sess-1');
    const t = montar(false);
    await tick();
    const abas = [...document.querySelectorAll('.aba')];
    expect(abas.map((a) => a.textContent?.trim())).toEqual(['Contexto', 'Arquivos', 'Navegador']);
    unmount(t.comp);
    fecharNav('srv-test::sess-1');
  });

  it('com a aba Navegador ativa, o header e a fileira de ações saem (o browser ganha a altura)', async () => {
    const { marcarNavAberto, fecharNav } = await import('../lib/navegadorPanel.svelte');
    marcarNavAberto('srv-test::sess-1');
    const t = montar(false);
    await tick();
    // Sessão com navegador que nunca escolheu aba entra DIRETO no Navegador (o agente pode ter
    // aberto com ela fora da tela): o header do painel (nome+estado) some; o <header class=nav-bar>
    // que fica é a barra de endereço DO NAVEGADOR, que é dele e tem que ficar.
    expect(ctxPanel.aba).toBe('navegador');
    expect(document.querySelector('.session-context .ctx-heading')).toBeNull();
    expect(document.querySelector('.ctx-actions')).toBeNull();
    ctxPanel.aba = 'contexto';
    await tick();
    expect(document.querySelector('.session-context .ctx-heading')).not.toBeNull();
    ctxPanel.aba = 'navegador';
    await tick();
    expect(document.querySelector('.session-context .ctx-heading')).toBeNull();
    unmount(t.comp);
    fecharNav('srv-test::sess-1');
  });

  it('aba Arquivos monta o FilesPanel e lista a sessao', async () => {
    localStorage.setItem('cp_servers', JSON.stringify([{ id: 'srv-test', label: 'T', baseUrl: 'http://x', token: 't' }]));
    const t = montar(false);
    await tick();
    const arq = [...document.querySelectorAll('.aba')][1] as HTMLButtonElement;
    arq.click();
    await tick();
    await tick();   // o onMount do FilesPanel -> recarregar -> listFiles
    expect(document.querySelector('.files-panel')).not.toBeNull();
    expect(listFiles).toHaveBeenCalledWith('sess-1', undefined, true, expect.objectContaining({ id: 'srv-test' }));
    unmount(t.comp);
  });
});

// Task 17: divisória redimensionável. O handle existe só com o painel ABERTO (recolhido não é
// largura), e o arrasto segue o MESMO contrato da Sidebar: pointerdown captura, move atualiza o
// store, soltar persiste. O happy-dom não implementa setPointerCapture — stub no teste, o
// comportamento real é do navegador (mesma limitação do teste da Sidebar).
describe('DesktopSessionContext — divisória redimensionável (task 17)', () => {
  it('handle presente com painel aberto, ausente com painel recolhido', async () => {
    const t = montar(false);
    await tick();
    const handle = document.querySelector<HTMLElement>('.ctx-resize-handle');
    expect(handle).not.toBeNull();
    expect(handle?.getAttribute('role')).toBe('separator');
    expect(handle?.getAttribute('aria-label')).toBe('Redimensionar painel de contexto');
    ctxPanel.recolhido = true;
    await tick();
    expect(document.querySelector('.ctx-resize-handle')).toBeNull();
    unmount(t.comp);
  });

  it('arrastar atualiza a largura e soltar persiste', async () => {
    Object.defineProperty(window, 'innerWidth', { value: 1600, configurable: true });
    const t = montar(false);
    await tick();
    const origCapture = HTMLElement.prototype.setPointerCapture;
    HTMLElement.prototype.setPointerCapture = vi.fn();
    try {
      const handle = document.querySelector<HTMLElement>('.ctx-resize-handle')!;
      // O arrasto mede a borda direita da COLUNA do painel, não a da janela (ele deixou de colar
      // no lado da tela). happy-dom não faz layout, então a medida vem daqui: 1600 = painel na
      // ponta direita, que é o arranjo padrão e o caso que este teste descreve.
      const painel = document.querySelector<HTMLElement>('.session-context')!;
      painel.getBoundingClientRect = () => ({ right: 1600 }) as DOMRect;
      // janela 1600 -> teto 560; clientX 1200 -> 400, dentro da faixa clampsa
      handle.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 1200, bubbles: true }));
      handle.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, clientX: 1200, bubbles: true }));
      expect(ctxPanel.largura).toBe(window.innerWidth - 1200);
      handle.dispatchEvent(new PointerEvent('pointerup', { pointerId: 1, bubbles: true }));
      expect(localStorage.getItem('cp_ctx_w')).toBe(String(window.innerWidth - 1200));
    } finally {
      HTMLElement.prototype.setPointerCapture = origCapture;
    }
    unmount(t.comp);
  });

  it('arrastar além do mínimo clampa no piso', async () => {
    Object.defineProperty(window, 'innerWidth', { value: 1600, configurable: true });
    const t = montar(false);
    await tick();
    const origCapture = HTMLElement.prototype.setPointerCapture;
    HTMLElement.prototype.setPointerCapture = vi.fn();
    try {
      const handle = document.querySelector<HTMLElement>('.ctx-resize-handle')!;
      handle.dispatchEvent(new PointerEvent('pointerdown', { pointerId: 1, clientX: 5000, bubbles: true }));
      handle.dispatchEvent(new PointerEvent('pointermove', { pointerId: 1, clientX: 5000, bubbles: true }));
      expect(ctxPanel.largura).toBe(LARGURA_MIN);
    } finally {
      HTMLElement.prototype.setPointerCapture = origCapture;
    }
    unmount(t.comp);
  });

  // A alca pode sair do DOM no meio do arrasto (recolher, cruzar os 820px, trocar de sessao) — o
  // pointerup fica sem destino e o flag do store (singleton) ficaria preso, fazendo a divisoria
  // redimensionar so com o cursor por cima. Recolher NAO desmonta o componente (a alca some por
  // {#if} interno) — o zero vem do $effect reagindo ao recolhido; os outros caminhos desmontam o
  // componente inteiro e o cleanup cobre.
  it('desmontar no meio do arrasto zera o resizing', async () => {
    const t = montar(false);
    await tick();
    ctxPanel.resizing = true;   // arrasto em curso
    unmount(t.comp);            // alca sai do DOM sem pointerup
    expect(ctxPanel.resizing).toBe(false);
  });

  it('recolher no meio do arrasto zera o resizing (a alca some por {#if}, sem desmontar)', async () => {
    const t = montar(false);
    await tick();
    ctxPanel.resizing = true;   // arrasto em curso
    ctxPanel.recolhido = true;  // recolheu: a alca sai do DOM, o componente fica montado
    await tick();
    expect(ctxPanel.resizing).toBe(false);
    unmount(t.comp);
  });
});

// O topo vivo (contexto/custo/tempo/turno) e o rodapé: o que o painel passou a dizer e que antes
// só existia na statusline do terminal ou dentro do modal de git.
describe('DesktopSessionContext — topo vivo e rodapé', () => {
  // A locale do CORE é outro runtime do paraglide (packages/core tem o seu): o overwriteGetLocale
  // do beforeEach só alcança o do frontend, e sem isto o Intl daqui formatava em en-US.
  beforeEach(() => configureLocale({ getLocale: () => 'pt' }));

  // Nome novo por teste: a aba escolhida vive num Map de MÓDULO por sessão (ABA_POR_SESSAO) e o
  // painel não remonta na troca — reusar o nome de outro teste herdava a aba dele.
  let n = 0;
  function montarCom(props: Record<string, unknown>) {
    const el = document.createElement('div');
    document.body.appendChild(el);
    const comp = mount(DesktopSessionContext, {
      target: el,
      props: { state: 'idle', sessionName: `topo-${++n}`, serverId: 'srv-test', toggleExterno: true, ...props },
    });
    return comp as never;
  }
  const txt = (sel: string) => (document.querySelector(sel)?.textContent ?? '').replace(/\s+/g, ' ').trim();

  it('sem medição de contexto: traço em vez de porcentagem, e o porquê escrito', async () => {
    const c = montarCom({ status: { raw: '' } });
    await tick();
    expect(txt('.agora-num')).toBe('—');
    expect(document.querySelector('.agora-num')?.classList.contains('vazio')).toBe(true);
    expect(txt('.sec-agora')).toContain('depois do primeiro turno');
    unmount(c);
  });

  it('custo sai na moeda da locale (pt-BR usa vírgula decimal, não ponto)', async () => {
    const c = montarCom({ status: { raw: '', costUsd: 16.37 } });
    await tick();
    expect(txt('.agora-custo')).toContain('16,37');
    unmount(c);
  });

  it('parada: conta desde o último evento (last_activity é quando ela parou)', async () => {
    const c = montarCom({ state: 'idle', session: { name: 's', state: 'idle', last_activity: Date.now() / 1000 - 600 } });
    await tick();
    expect(txt('.agora-linha')).toContain('parada há 10min');
    unmount(c);
  });

  it('trabalhando não ganha relógio próprio: a duração do turno já está no header', async () => {
    const c = montarCom({
      state: 'working',
      session: { name: 's', state: 'working', last_activity: Date.now() / 1000 - 600 },
    });
    await tick();
    expect(txt('.agora-linha')).not.toContain('parada');
    expect(txt('.agora-linha')).not.toContain('10min');
    unmount(c);
  });

  it('repositório mostra o tamanho do diff, não só que ele existe', async () => {
    const c = montarCom({
      status: { raw: '', repo: 'hangar', branch: 'main', dirty: true },
      session: { name: 's', state: 'idle', git_added: 980, git_removed: 84, git_dirty: 28 },
    });
    await tick();
    expect(txt('.repo-diff')).toBe('+980 −84 28 arq.');
    unmount(c);
  });

  it('sem numstat (repo sem commit) cai no texto antigo em vez de "+0 −0"', async () => {
    const c = montarCom({
      status: { raw: '', repo: 'hangar', branch: 'main', dirty: true },
      session: { name: 's', state: 'idle' },
    });
    await tick();
    expect(document.querySelector('.repo-diff')).toBeNull();
    expect(txt('.sec-break')).toContain('alterações locais');
    unmount(c);
  });

  it('fila só aparece com prompt pendente, e não conta o que o transcript já confirmou', async () => {
    const semFila = montarCom({ events: [{ kind: 'user_msg', id: 'queued-1', queued_confirmed: true }] });
    await tick();
    expect(document.querySelector('.rodape-fila')).toBeNull();
    unmount(semFila);

    document.body.innerHTML = '';
    const comFila = montarCom({
      events: [
        { kind: 'user_msg', id: 'queued-1', queued_confirmed: true },
        { kind: 'user_msg', id: 'queued-2' },
        { kind: 'user_msg', id: 'queued-3' },
      ],
    });
    await tick();
    expect(txt('.rodape-fila')).toBe('2 na fila');
    unmount(comFila);
  });

  it('aviso de recarregar: faixa com botão, e só com motivo E handler', async () => {
    const semMotivo = montarCom({ onRecarregar: () => {} });
    await tick();
    expect(document.querySelector('.ctx-aviso')).toBeNull();
    unmount(semMotivo);

    document.body.innerHTML = '';
    let chamou = 0;
    const comMotivo = montarCom({ recarregarMotivo: 'config', onRecarregar: () => (chamou += 1) });
    await tick();
    const btn = document.querySelector<HTMLButtonElement>('.ctx-aviso-btn');
    expect(btn).not.toBeNull();
    btn!.click();
    expect(chamou).toBe(1);
    unmount(comMotivo);
  });

  it('aviso de recarregar não é clicável fora de ociosa', async () => {
    const c = montarCom({ recarregarMotivo: 'config', onRecarregar: () => {}, recarregarBloqueado: true });
    await tick();
    expect(document.querySelector<HTMLButtonElement>('.ctx-aviso-btn')!.disabled).toBe(true);
    unmount(c);
  });

  it('mais alterados: três maiores primeiro, binário de fora, e a linha abre o arquivo', async () => {
    const abertos: string[] = [];
    const c = montarCom({
      status: { raw: '', repo: 'hangar', branch: 'main', dirty: true },
      session: { name: 's', state: 'idle', git_added: 115, git_removed: 5, git_dirty: 4 },
      onOpenGit: () => {},
      onAbrirArquivo: (p: string) => abertos.push(p),
    });
    await tick();
    await Promise.resolve();
    await tick();
    const linhas = [...document.querySelectorAll('.arq-linha')];
    expect(linhas.map((l) => l.querySelector('.arq-path')!.textContent)).toEqual([
      '…/components/Grande.svelte', 'docs/meio.md', 'src/a.ts',
    ]);
    (linhas[0] as HTMLButtonElement).click();
    expect(abertos).toEqual(['src/components/Grande.svelte']);
    unmount(c);
  });

  it('repositório limpo esconde a lista em vez de deixar os arquivos de antes do commit', async () => {
    const props = {
      status: { raw: '', repo: 'hangar', branch: 'main', dirty: true },
      session: { name: 's', state: 'idle', git_added: 115, git_removed: 5, git_dirty: 4 },
      onOpenGit: () => {},
    };
    const c = montarCom(props);
    await tick();
    await Promise.resolve();
    await tick();
    expect(document.querySelectorAll('.arq-linha').length).toBe(3);

    // Commitou: o backend passa a mandar 0 — que é número, não ausente, então `diffRepo` continua
    // existindo e o gate precisa olhar o conteúdo dele.
    unmount(c);
    document.body.innerHTML = '';
    const limpo = montarCom({
      ...props,
      status: { raw: '', repo: 'hangar', branch: 'main', dirty: false },
      session: { name: 's', state: 'idle', git_added: 0, git_removed: 0, git_dirty: 0 },
    });
    await tick();
    await Promise.resolve();
    await tick();
    expect(document.querySelectorAll('.arq-linha').length).toBe(0);
    unmount(limpo);
  });

  // O Chat não passa onOpenPair para a linha do orquestrador: sem ele, nada de "parear".
  it('sem onOpenPair e sem pares, não oferece parear', async () => {
    const sem = montarCom({});
    await tick();
    expect(document.querySelector(`[aria-label="${m.ctx_parear_outra()}"]`)).toBeNull();
    unmount(sem);
    const com = montarCom({ onOpenPair: vi.fn() });
    await tick();
    expect(document.querySelector(`[aria-label="${m.ctx_parear_outra()}"]`)).not.toBeNull();
    unmount(com);
  });

  it('execução fica no rodapé, fora do scroller', async () => {
    const c = montarCom({ provider: 'claude', serverLabel: 'Notebook' });
    await tick();
    expect(txt('.ctx-rodape')).toContain('Claude · Notebook');
    expect(document.querySelector('.ctx-scroll .ctx-rodape')).toBeNull();
    unmount(c);
  });
});
