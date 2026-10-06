import { inkColor } from '@hangar/core';

// Medidas dos mods são em células de terminal: largura em `ch`, altura em linhas (`lh`).
const PERCENT = /^\d+(\.\d+)?%$/;
const cols = (v: unknown): string | null =>
  typeof v === 'number' && Number.isFinite(v) ? `${v}ch` : typeof v === 'string' && PERCENT.test(v) ? v : null;
const lines = (v: unknown): string | null => (typeof v === 'number' && Number.isFinite(v) ? `${v}lh` : null);

// Os valores vêm do mod e vão para um `style`: só palavras de CSS (`flex-start`, `row`), números e
// os de cima. Qualquer coisa que feche a declaração ou abra `url(` cai fora.
const SAFE_VALUE = /^[\w#%.,\s()-]*$/;

function css(parts: Record<string, string | number | null | undefined | false>): string {
  return Object.entries(parts)
    .filter(([, v]) => v !== null && v !== undefined && v !== false && v !== '')
    .filter(([, v]) => typeof v === 'number' || (SAFE_VALUE.test(v as string) && !/url\(/i.test(v as string)))
    .map(([k, v]) => `${k}:${v}`)
    .join(';');
}

type Props = Record<string, unknown>;

/** `width` em colunas que alcança a largura do lugar ocupa o lugar inteiro: o mod desenhou para a coluna do
 *  terminal, e o app pode ser mais largo. Menor continua teto. Sem a largura do lugar (servidor antigo), teto. */
export function fillsPlace(width: unknown, place: number | null | undefined): boolean {
  return typeof width === 'number' && typeof place === 'number' && place > 0 && width >= place;
}

/** `Box` do Ink em CSS de flexbox. O padrão do Ink é linha, não coluna. */
export function boxStyle(p: Props, place?: number | null): string {
  const pick = (...keys: string[]) => keys.map((k) => p[k]).find((v) => typeof v === 'number');
  const border = typeof p.borderStyle === 'string' && p.borderStyle;
  // `absolute` sai do fluxo e pinta por cima, preso ao Box pai (que é `position: relative`) e recortado pelo lugar.
  const absolute = p.position === 'absolute';
  return css({
    display: p.display === 'none' ? 'none' : 'flex',
    position: absolute ? 'absolute' : null,
    top: absolute ? lines(p.top) : null,
    bottom: absolute ? lines(p.bottom) : null,
    left: absolute ? cols(p.left) : null,
    right: absolute ? cols(p.right) : null,
    'z-index': absolute ? 1 : null,
    'flex-direction': typeof p.flexDirection === 'string' ? p.flexDirection : 'row',
    'flex-grow': typeof p.flexGrow === 'number' ? p.flexGrow : null,
    'flex-shrink': typeof p.flexShrink === 'number' ? p.flexShrink : null,
    'flex-wrap': typeof p.flexWrap === 'string' ? p.flexWrap : null,
    'align-items': typeof p.alignItems === 'string' ? p.alignItems : null,
    'align-self': typeof p.alignSelf === 'string' ? p.alignSelf : null,
    'justify-content': typeof p.justifyContent === 'string' ? p.justifyContent : null,
    'column-gap': cols(pick('columnGap', 'gap')),
    'row-gap': lines(pick('rowGap', 'gap')),
    // Largura fixa do terminal vira teto (no celular a coluna é mais estreita que a do pane), salvo quando ela
    // alcança a largura do lugar: aí o mod quis a linha inteira.
    'max-width': fillsPlace(p.width, place) ? null : cols(p.width),
    width: typeof p.width === 'number' ? '100%' : cols(p.width),
    'min-width': cols(p.minWidth) ?? '0',
    'padding-top': lines(pick('paddingTop', 'paddingY', 'padding')),
    'padding-bottom': lines(pick('paddingBottom', 'paddingY', 'padding')),
    'padding-left': cols(pick('paddingLeft', 'paddingX', 'padding')),
    'padding-right': cols(pick('paddingRight', 'paddingX', 'padding')),
    'margin-top': lines(pick('marginTop', 'marginY', 'margin')),
    'margin-bottom': lines(pick('marginBottom', 'marginY', 'margin')),
    'margin-left': cols(pick('marginLeft', 'marginX', 'margin')),
    'margin-right': cols(pick('marginRight', 'marginX', 'margin')),
    // No terminal as células do cartão substituem as de baixo: sem cor própria, ele leva o fundo opaco do lugar
    // (`--plugin-place-bg`, da faixa e do painel), senão o texto dele se embaralha com o da linha.
    background: inkColor(p.backgroundColor) ?? (absolute ? 'var(--plugin-place-bg)' : null),
    border: border ? `1px solid ${inkColor(p.borderColor) ?? 'var(--border-default)'}` : null,
    'border-radius': border === 'round' ? '6px' : null,
    overflow: p.overflow === 'hidden' ? 'hidden' : null,
  });
}

/** `Text` do Ink: cor, ênfase e corte. `dimColor` é opacidade, como no terminal. */
export function textStyle(p: Props): string {
  const fg = inkColor(p.color);
  const bg = inkColor(p.backgroundColor);
  const inverse = p.inverse === true;
  const wrap = typeof p.wrap === 'string' ? p.wrap : 'wrap';
  const truncate = wrap.startsWith('truncate') || wrap === 'end' || wrap === 'middle';
  const deco = [p.underline === true && 'underline', p.strikethrough === true && 'line-through'].filter(Boolean);
  return css({
    color: inverse ? (bg ?? 'var(--bg-base)') : fg,
    background: inverse ? (fg ?? 'var(--text-primary)') : bg,
    'font-weight': p.bold === true ? 700 : null,
    'font-style': p.italic === true ? 'italic' : null,
    'text-decoration': deco.length ? deco.join(' ') : null,
    opacity: p.dimColor === true ? 0.6 : null,
    'white-space': truncate ? 'pre' : 'pre-wrap',
    overflow: truncate ? 'hidden' : null,
    'text-overflow': truncate ? 'ellipsis' : null,
    'min-width': truncate ? '0' : null,
  });
}

/** Rótulo de `Button` sob o hover do escopo: o conjunto de estilo de texto da API (cor, fundo, esmaecido,
 *  negrito, itálico, sublinhado, riscado); a pílula continua a mesma. */
export function buttonStyle(p: Props): string {
  const deco = [p.underline === true && 'underline', p.strikethrough === true && 'line-through'].filter(Boolean);
  return css({
    color: inkColor(p.color),
    background: inkColor(p.backgroundColor),
    'font-weight': p.bold === true ? 700 : null,
    'font-style': p.italic === true ? 'italic' : null,
    'text-decoration': deco.length ? deco.join(' ') : null,
    opacity: p.dimColor === true ? 0.6 : null,
  });
}
