import { describe, expect, it } from 'vitest';
import { boxStyle, buttonStyle, fillsPlace } from './pluginUiStyle';

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
