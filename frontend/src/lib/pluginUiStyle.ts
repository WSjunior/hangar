import { inkColor } from '@hangar/core';

// Medidas dos mods são em células de terminal: largura em `ch`, altura em linhas (`lh`).
const cols = (v: unknown): string | null =>
  typeof v === 'number' ? `${v}ch` : typeof v === 'string' && v ? v : null;
const lines = (v: unknown): string | null => (typeof v === 'number' ? `${v}lh` : null);

function css(parts: Record<string, string | number | null | undefined | false>): string {
  return Object.entries(parts)
    .filter(([, v]) => v !== null && v !== undefined && v !== false && v !== '')
    .map(([k, v]) => `${k}:${v}`)
    .join(';');
}

type Props = Record<string, unknown>;

/** `Box` do Ink em CSS de flexbox. O padrão do Ink é linha, não coluna. */
export function boxStyle(p: Props): string {
  const pick = (...keys: string[]) => keys.map((k) => p[k]).find((v) => typeof v === 'number');
  const border = typeof p.borderStyle === 'string' && p.borderStyle;
  return css({
    display: p.display === 'none' ? 'none' : 'flex',
    'flex-direction': typeof p.flexDirection === 'string' ? p.flexDirection : 'row',
    'flex-grow': typeof p.flexGrow === 'number' ? p.flexGrow : null,
    'flex-shrink': typeof p.flexShrink === 'number' ? p.flexShrink : null,
    'flex-wrap': typeof p.flexWrap === 'string' ? p.flexWrap : null,
    'align-items': typeof p.alignItems === 'string' ? p.alignItems : null,
    'align-self': typeof p.alignSelf === 'string' ? p.alignSelf : null,
    'justify-content': typeof p.justifyContent === 'string' ? p.justifyContent : null,
    'column-gap': cols(pick('columnGap', 'gap')),
    'row-gap': lines(pick('rowGap', 'gap')),
    // Largura fixa do terminal vira teto: no celular a coluna é mais estreita que a do pane.
    'max-width': cols(p.width),
    width: typeof p.width === 'number' ? '100%' : typeof p.width === 'string' ? p.width : null,
    'min-width': cols(p.minWidth) ?? '0',
    'padding-top': lines(pick('paddingTop', 'paddingY', 'padding')),
    'padding-bottom': lines(pick('paddingBottom', 'paddingY', 'padding')),
    'padding-left': cols(pick('paddingLeft', 'paddingX', 'padding')),
    'padding-right': cols(pick('paddingRight', 'paddingX', 'padding')),
    'margin-top': lines(pick('marginTop', 'marginY', 'margin')),
    'margin-bottom': lines(pick('marginBottom', 'marginY', 'margin')),
    'margin-left': cols(pick('marginLeft', 'marginX', 'margin')),
    'margin-right': cols(pick('marginRight', 'marginX', 'margin')),
    background: inkColor(p.backgroundColor),
    border: border ? `1px solid ${inkColor(p.borderColor) ?? 'var(--border)'}` : null,
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
    color: inverse ? (bg ?? 'var(--bg)') : fg,
    background: inverse ? (fg ?? 'var(--text)') : bg,
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
