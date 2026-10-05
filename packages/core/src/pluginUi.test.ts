import { describe, expect, it } from 'vitest';
import { activePaneId, buttonKey, followLocalTab, isMissingRoute, tabFollowsServer, decodeRaster, inkColor, isEmptyBand, parsePluginToast, parsePluginUi, textOf } from './pluginUi';
import amostras from './__fixtures__/plugin-ui-arvores.json';

function cells(words: number[]): string {
  const bytes = new Uint8Array(new Uint32Array(words).buffer);
  return btoa(String.fromCharCode(...bytes));
}

describe('decodeRaster', () => {
  it('lê código, frente e fundo por célula, com a cor padrão como null', () => {
    const grid = decodeRaster(cells([0x2501, 0x5aa6ff, 0x01000000, 0x41, 0x01000000, 0x112233]), 2, 1);
    expect(grid).toEqual([[
      { ch: '━', fg: '#5aa6ff', bg: null },
      { ch: 'A', fg: null, bg: '#112233' },
    ]]);
  });

  it('para no fim dos dados em vez de inventar célula', () => {
    expect(decodeRaster(cells([0x41, 0, 0]), 3, 1)[0]).toHaveLength(1);
  });

  it('não cria linha além dos dados que chegaram, mesmo com `rows` enorme', () => {
    expect(decodeRaster(cells([0x41, 0, 0, 0x42, 0, 0, 0x43, 0, 0]), 2, 1_000)).toHaveLength(2);
    expect(decodeRaster(cells([0x41, 0, 0]), 0, 1_000)).toEqual([]);
  });
});

describe('isEmptyBand', () => {
  it('trata ausência e o marcador do engine como faixa vazia', () => {
    expect(isEmptyBand(null)).toBe(true);
    expect(isEmptyBand({ type: 'engine' })).toBe(true);
    expect(isEmptyBand({ type: 'Box', children: [] })).toBe(false);
  });
});

it('inkColor traduz nomes do Ink e mantém hex', () => {
  expect(inkColor('redBright')).toBe('#f14c4c');
  expect(inkColor('#5aa6ff')).toBe('#5aa6ff');
  expect(inkColor(undefined)).toBeNull();
});

it('inkColor recusa texto que injetaria CSS', () => {
  expect(inkColor('red;position:fixed')).toBeNull();
  expect(inkColor('url(https://x)')).toBeNull();
  expect(inkColor('rgb(1, 2, 3)')).toBe('rgb(1, 2, 3)');
});

it('textOf junta só texto e número', () => {
  expect(textOf(['a', 1, { type: 'Text' }, null])).toBe('a1');
});

describe('parsePluginUi', () => {
  it('lê faixa e painéis e descarta painel sem id', () => {
    const s = parsePluginUi({ above: { type: 'Box' }, panes: [
      { id: 'review-mr', title: 'Review !577', placement: 'dock', columns: 72, tree: { type: 'Box' } },
      { title: 'sem id' },
    ] });
    expect(s.above).toEqual({ type: 'Box' });
    expect(s.panes.map((p) => p.id)).toEqual(['review-mr']);
    expect(s.panes[0].placement).toBe('dock');
  });

  it('aceita o formato antigo, só com a faixa', () => {
    expect(parsePluginUi({ above: null })).toEqual({ above: null, panes: [], shownId: undefined, columns: null, source: null });
  });

  it('placement desconhecido vira inline', () => {
    expect(parsePluginUi({ panes: [{ id: 'a', placement: 'x' }] }).panes[0].placement).toBe('inline');
  });
});

describe('parsePluginToast', () => {
  it('lê o aviso e o mod que o emitiu', () => {
    expect(parsePluginToast({ id: 'ab-1', text: 'Jenkins configurado.', plugin: 'demo', timeoutMs: 9000 }))
      .toEqual({ id: 'ab-1', text: 'Jenkins configurado.', plugin: 'demo', timeoutMs: 9000 });
  });

  it('sem id, sem texto ou sem prazo não é aviso', () => {
    expect(parsePluginToast({ text: 'oi', timeoutMs: 4000 })).toBeNull();
    expect(parsePluginToast({ id: 'ab-3', text: '  ', timeoutMs: 4000 })).toBeNull();
    expect(parsePluginToast({ id: 'ab-4', text: 'oi' })).toBeNull();
    expect(parsePluginToast({ id: 'ab-5', text: 'oi', timeoutMs: 0 })).toBeNull();
    expect(parsePluginToast(null)).toBeNull();
  });
});

it('buttonKey só para Button com key em texto', () => {
  expect(buttonKey({ type: 'Button', props: { key: 'cp-1' } })).toBe('cp-1');
  expect(buttonKey({ type: 'Button', props: {} })).toBeNull();
  expect(buttonKey({ type: 'Text', props: { key: 'x' } })).toBeNull();
});

describe('parsePluginUi: campos novos da fase 1', () => {
  const evento = (extra: Record<string, unknown>) => ({
    above: amostras.faixaPm,
    panes: amostras.rolPm.panes.map((p) => ({ ...p, placement: 'dock', columns: 58, tree: { type: 'engine', ref: 0 } })),
    ...extra,
  });

  it('lê shown_id, columns e source quando o servidor manda', () => {
    const s = parsePluginUi(evento({ shown_id: 'pm-mock-mr', columns: 110, source: 'surface' }));
    expect([s.shownId, s.columns, s.source]).toEqual(['pm-mock-mr', 110, 'surface']);
    expect(s.panes.map((p) => p.id)).toEqual(['pm-mock-pm', 'pm-mock-mr', 'pm-mock-jenkins']);
  });

  it('servidor de hoje: sem os campos, shownId fica undefined e o resto null', () => {
    const s = parsePluginUi(evento({}));
    expect(s.shownId).toBeUndefined();
    expect(s.columns).toBeNull();
    expect(s.source).toBeNull();
  });

  it('shown_id null quer dizer "sem painel"; valores estranhos valem como ausentes', () => {
    expect(parsePluginUi({ shown_id: null }).shownId).toBeNull();
    const s = parsePluginUi({ shown_id: 7, columns: -3, source: 'mobile' });
    expect([s.shownId, s.columns, s.source]).toEqual([undefined, null, null]);
    expect(parsePluginUi({ shown_id: '' }).shownId).toBeUndefined();
  });

  it('o hover da árvore real fica no nó, fora de props', () => {
    const linha = parsePluginUi(evento({})).above as unknown as { children: { children?: { hover?: unknown; props?: Record<string, unknown> }[] }[] };
    const cartao = linha.children[1].children![2];
    expect(cartao.hover).toEqual({ display: 'flex' });
    expect(cartao.props).not.toHaveProperty('hover');
  });
});

describe('aba ativa', () => {
  const ids = ['pm-mock-pm', 'pm-mock-mr', 'pm-mock-jenkins'];

  it('segue o shown_id quando ele nomeia um painel da lista', () => {
    expect(tabFollowsServer(ids, 'pm-mock-pm')).toBe(true);
    expect(activePaneId(ids, 'pm-mock-pm', 'pm-mock-mr')).toBe('pm-mock-pm');
  });

  it('shown_id de painel que ainda não chegou: vale a escolha local, nunca nada', () => {
    expect(tabFollowsServer(ids, 'pm-mock-novo')).toBe(false);
    expect(activePaneId(ids, 'pm-mock-novo', 'pm-mock-mr')).toBe('pm-mock-mr');
    expect(activePaneId(ids, 'pm-mock-novo', null)).toBe('pm-mock-jenkins');
  });

  it('sem shown_id (servidor antigo), escolha local; sem ela, o último aberto; sem painel, null', () => {
    expect(activePaneId(ids, undefined, 'pm-mock-pm')).toBe('pm-mock-pm');
    expect(activePaneId(ids, null, null)).toBe('pm-mock-jenkins');
    expect(activePaneId([], 'x', 'y')).toBeNull();
  });

  it('escolha local que não está mais na lista cai no último aberto', () => {
    expect(activePaneId(ids, undefined, 'fechado')).toBe('pm-mock-jenkins');
  });
});

describe('escolha local da aba', () => {
  it('começa no último painel aberto', () => {
    expect(followLocalTab([], ['a', 'b', 'c'], null)).toBe('c');
  });

  it('sobrevive a um redesenho sem painel novo', () => {
    expect(followLocalTab(['a', 'b', 'c'], ['a', 'b', 'c'], 'a')).toBe('a');
  });

  it('painel que acaba de abrir vai para a frente, como no terminal', () => {
    expect(followLocalTab(['a', 'b'], ['a', 'b', 'd'], 'a')).toBe('d');
  });

  it('fechado o escolhido, fica o vizinho anterior; sem anterior, o seguinte', () => {
    expect(followLocalTab(['a', 'b', 'c'], ['a', 'c'], 'b')).toBe('a');
    expect(followLocalTab(['a', 'b'], ['b'], 'a')).toBe('b');
    expect(followLocalTab(['a'], [], 'a')).toBeNull();
  });
});

describe('rota ausente', () => {
  it('404 e 405 são servidor anterior à rota; 409 e erro sem status não são', () => {
    expect(isMissingRoute(Object.assign(new Error('Not Found'), { status: 404 }))).toBe(true);
    expect(isMissingRoute(Object.assign(new Error('Method Not Allowed'), { status: 405 }))).toBe(true);
    expect(isMissingRoute(Object.assign(new Error('x'), { status: 409, code: 'erro_mod_dialogo_aberto' }))).toBe(false);
    expect(isMissingRoute(new Error('rede'))).toBe(false);
    expect(isMissingRoute(null)).toBe(false);
  });
});
