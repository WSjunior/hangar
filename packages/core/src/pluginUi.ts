/** Árvore de elementos que os mods do Claude Code desenham (`ui.render`), como o engine a entrega
 *  a uma superfície remota. O Hangar não conhece mod nenhum: só traduz estes elementos. */
export interface PluginElement {
  type: string;
  props?: Record<string, unknown>;
  children?: PluginNode[];
}

export type PluginNode = PluginElement | string | number | boolean | null | undefined;

export interface RasterCell {
  ch: string;
  /** `#rrggbb`, ou null para a cor padrão da superfície. */
  fg: string | null;
  bg: string | null;
}

/** Faixa sem nada para mostrar: ninguém desenhou, ou só o marcador do próprio engine. */
export function isEmptyBand(tree: PluginNode): boolean {
  return !tree || typeof tree !== 'object' || tree.type === 'engine';
}

const B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

// Sem `atob`: o core também roda no app Expo, onde ele não é garantido.
function base64Bytes(text: string): Uint8Array {
  const clean = text.replace(/[^A-Za-z0-9+/]/g, '');
  const out = new Uint8Array(Math.floor((clean.length * 3) / 4));
  let bits = 0;
  let acc = 0;
  let n = 0;
  for (const c of clean) {
    acc = (acc << 6) | B64.indexOf(c);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[n++] = (acc >> bits) & 0xff;
    }
  }
  return out.subarray(0, n);
}

// Bit 24 sozinho é a cor padrão do terminal; o resto é 0x00RRGGBB.
function cellColor(v: number): string | null {
  if (v & 0x01000000) return null;
  return `#${(v & 0xffffff).toString(16).padStart(6, '0')}`;
}

/** Células do `Raster`: base64 de triplas u32 little-endian `[código, frente, fundo]`, por linha. */
export function decodeRaster(cells: string, columns: number, rows: number): RasterCell[][] {
  const bytes = base64Bytes(cells);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const grid: RasterCell[][] = [];
  for (let r = 0; r < rows; r++) {
    const row: RasterCell[] = [];
    for (let c = 0; c < columns; c++) {
      const at = (r * columns + c) * 12;
      if (at + 12 > bytes.length) break;
      const code = view.getUint32(at, true);
      row.push({
        ch: code >= 32 ? String.fromCharCode(code) : ' ',
        fg: cellColor(view.getUint32(at + 4, true)),
        bg: cellColor(view.getUint32(at + 8, true)),
      });
    }
    grid.push(row);
  }
  return grid;
}

// Nomes de cor do Ink (o terminal do Claude Code) que não existem em CSS ou lá têm outro tom.
const INK_COLORS: Record<string, string> = {
  black: '#000000',
  red: '#cd3131',
  green: '#0dbc79',
  yellow: '#e5e510',
  blue: '#2472c8',
  magenta: '#bc3fbc',
  cyan: '#11a8cd',
  white: '#e5e5e5',
  gray: '#808080',
  grey: '#808080',
  blackBright: '#666666',
  redBright: '#f14c4c',
  greenBright: '#23d18b',
  yellowBright: '#f5f543',
  blueBright: '#3b8eea',
  magentaBright: '#d670d6',
  cyanBright: '#29b8db',
  whiteBright: '#ffffff',
};

/** Cor de um `Text`/`Box` em CSS: hex e `rgb()` passam; nome do Ink vira o tom do terminal. */
export function inkColor(value: unknown): string | null {
  if (typeof value !== 'string' || !value) return null;
  return INK_COLORS[value] ?? value;
}

/** Texto direto dos filhos (strings e números), na ordem. */
export function textOf(children: PluginNode[] | undefined): string {
  return (children ?? []).map((c) => (typeof c === 'string' || typeof c === 'number' ? String(c) : '')).join('');
}
