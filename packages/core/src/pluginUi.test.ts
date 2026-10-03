import { describe, expect, it } from 'vitest';
import { decodeRaster, inkColor, isEmptyBand, textOf } from './pluginUi';

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

it('textOf junta só texto e número', () => {
  expect(textOf(['a', 1, { type: 'Text' }, null])).toBe('a1');
});
