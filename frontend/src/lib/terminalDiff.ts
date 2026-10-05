import type { Span } from '@hangar/core';

/** Linhas de um arquivo novo antes do "… +N linhas": o mesmo corte do Claude Code. */
export const WRITE_SHOWN = 10;
/** Linhas de uma edição antes do "… +N linhas"; o botão expande o resto. */
export const EDIT_SHOWN = 40;

export interface Piece { content: string; color: string | undefined; mark: boolean }

/** Corta a linha onde muda a cor da sintaxe ou começa/termina um trecho trocado. */
export function piecesOf(text: string, tokens: { content: string; color?: string }[] | null, spans: Span[]): Piece[] {
  // Caminho rápido (toda removida e a maioria das de contexto): sem realce nem trecho marcado, a linha é uma peça só.
  if (text && !tokens?.length && !spans.length) return [{ content: text, color: undefined, mark: false }];
  const colors: { from: number; to: number; color?: string }[] = [];
  let pos = 0;
  for (const t of tokens ?? []) { colors.push({ from: pos, to: pos + t.content.length, color: t.color }); pos += t.content.length; }
  // Realce que não fecha com o texto colaria a cor no lugar errado: melhor sem cor.
  const usable = pos === text.length ? colors : [];
  const cuts = new Set<number>([0, text.length]);
  for (const c of usable) cuts.add(c.to);
  for (const [s, e] of spans) { cuts.add(Math.min(s, text.length)); cuts.add(Math.min(e, text.length)); }
  const points = [...cuts].sort((a, b) => a - b);
  const out: Piece[] = [];
  for (let i = 0; i + 1 < points.length; i++) {
    const from = points[i], to = points[i + 1];
    out.push({
      content: text.slice(from, to),
      color: usable.find((c) => c.from <= from && to <= c.to)?.color,
      mark: spans.some(([s, e]) => s <= from && to <= e),
    });
  }
  return out;
}

/** O último segmento não vazio do caminho (barra comum ou invertida); `fallback` quando não sobra nenhum. */
export function fileName(path: string, fallback = path): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? fallback;
}

/** O que separa um trecho do anterior: o intervalo no mesmo arquivo, o nome do arquivo quando ele muda. Sem caminhos
 * (patch do resultado) todo trecho é do mesmo arquivo. A regra é a do `between` do nativo. */
export function betweenHunks(paths: string[] | undefined, hunk: number): { kind: 'gap' } | { kind: 'file'; name: string; path: string } {
  const prev = paths?.[hunk - 1];
  const cur = paths?.[hunk];
  if (cur === undefined || prev === cur) return { kind: 'gap' };
  return { kind: 'file', name: fileName(cur), path: cur };
}
