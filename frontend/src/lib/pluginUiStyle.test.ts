import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { boxStyle, buttonStyle, fillsPlace, textStyle } from './pluginUiStyle';

describe('tokens do tema nos estilos dos mods', () => {
  // `var()` de um token que o tema não define vira o valor inicial: a borda some (`border-style: none`), a cor
  // volta à herdada. Por isso os estilos dos mods só usam token declarado no app.css ou nos próprios lugares dos
  // mods (o `--plugin-place-bg` da faixa e do painel).
  const ler = (caminho: string) => readFileSync(new URL(caminho, import.meta.url), 'utf8');
  const declarados = (texto: string) => [...texto.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]);
  const usados = (texto: string) => [...texto.matchAll(/var\(\s*(--[\w-]+)/g)].map((m) => m[1]);
  const arquivos = ['./pluginUiStyle.ts', '../components/PluginNode.svelte', '../components/PluginInput.svelte',
    '../components/PluginBand.svelte', '../components/PluginPane.svelte'];
  const definidos = new Set([...declarados(ler('../app.css')), ...arquivos.flatMap((a) => declarados(ler(a)))]);

  it.each(arquivos)('%s só usa tokens definidos', (arquivo) => {
    expect(usados(ler(arquivo)).filter((t) => !definidos.has(t))).toEqual([]);
  });

  it('Box com borda e sem borderColor usa a borda do tema', () => {
    expect(boxStyle({ borderStyle: 'round' })).toContain('border:1px solid var(--border-default)');
    expect(boxStyle({ borderStyle: 'single', borderColor: '#5aa6ff' })).toContain('border:1px solid #5aa6ff');
  });
});

describe('largura em colunas', () => {
  it('width que alcança a largura do lugar ocupa o lugar inteiro', () => {
    expect(fillsPlace(110, 110)).toBe(true);
    expect(fillsPlace(120, 110)).toBe(true);
    const s = boxStyle({ width: 110 }, 110);
    expect(s).toContain('width:100%');
    expect(s).not.toContain('max-width');
  });

  it('width menor que o lugar continua como teto em colunas', () => {
    expect(boxStyle({ width: 24 }, 58)).toContain('max-width:24ch');
  });

  it('sem a largura do lugar (servidor de hoje), nada muda', () => {
    expect(fillsPlace(110, null)).toBe(false);
    expect(fillsPlace(110, undefined)).toBe(false);
    expect(fillsPlace('50%', 110)).toBe(false);
    expect(boxStyle({ width: 110 })).toContain('max-width:110ch');
  });
});

describe('posição absoluta', () => {
  it('sai do fluxo, com deslocamento em células, por cima dos vizinhos', () => {
    const s = boxStyle({ position: 'absolute', top: 1, left: 2, display: 'none' });
    expect(s).toContain('position:absolute');
    expect(s).toContain('top:1lh');
    expect(s).toContain('left:2ch');
    expect(s).toContain('z-index:1');
    expect(s).toContain('display:none');
  });

  it('sem backgroundColor, o cartão ganha o fundo opaco do lugar e tapa a linha de baixo', () => {
    expect(boxStyle({ position: 'absolute', top: 1 })).toContain('background:var(--plugin-place-bg)');
    expect(boxStyle({ position: 'absolute', backgroundColor: '#30363d' })).toContain('background:#30363d');
    expect(boxStyle({ flexDirection: 'row' })).not.toContain('background');
  });

  it('deslocamento negativo passa; sem position, top e left são ignorados', () => {
    expect(boxStyle({ position: 'absolute', top: -1, right: 0 })).toContain('top:-1lh');
    expect(boxStyle({ top: 1, left: 2 })).not.toMatch(/(^|;)(top|left|position):/);
  });
});

describe('buttonStyle', () => {
  it('leva cor e negrito do rótulo e nada mais quando só isso vem', () => {
    expect(buttonStyle({ color: '#e8a33d', bold: true })).toBe('color:#e8a33d;font-weight:700');
    expect(buttonStyle({ label: 'x' })).toBe('');
  });

  it('aplica o conjunto inteiro de estilo de texto: fundo, itálico, sublinhado, riscado e esmaecido', () => {
    const s = buttonStyle({ color: '#e8a33d', backgroundColor: '#112233', italic: true, underline: true, strikethrough: true, dimColor: true });
    expect(s).toContain('color:#e8a33d');
    expect(s).toContain('background:#112233');
    expect(s).toContain('font-style:italic');
    expect(s).toContain('text-decoration:underline line-through');
    expect(s).toContain('opacity:0.6');
  });
});

describe('textStyle', () => {
  it('Text de fora com corte: uma linha só, recortada com reticências', () => {
    const s = textStyle({ wrap: 'truncate-end' });
    for (const parte of ['white-space:pre', 'overflow:hidden', 'text-overflow:ellipsis', 'min-width:0']) expect(s).toContain(parte);
    expect(textStyle({})).toContain('white-space:pre-wrap');
  });

  it('Text dentro de Text ignora o próprio wrap e herda o do Text de fora, como o trecho do motor', () => {
    for (const wrap of [undefined, 'wrap', 'truncate-end', 'truncate-middle']) {
      const s = textStyle({ wrap, color: '#3fb97a' }, true);
      expect(s).toBe('color:#3fb97a');
    }
  });
});
