// Página publicada pelo agente (tool html_render do MCP hangar), desenhada no lugar da chamada.
export type HtmlPageRef = { id: string; title: string; height: number | null; heights: Record<string, number> };
export type PageFetchState = 'loading' | 'ready' | 'error' | 'expired';

const MIN = 80;
const MAX = 2000;
const FALLBACK = 240;
// Claude: mcp__hangar__html_render. O nome no Codex é conferido no passo de uso real.
const NAMES = new Set(['mcp__hangar__html_render']);

export function isHtmlRenderTool(name: string | null | undefined): boolean {
  return !!name && NAMES.has(name);
}

export function htmlPageFromResult(toolName: string | null | undefined, result: string | null | undefined): HtmlPageRef | null {
  if (!isHtmlRenderTool(toolName) || !result) return null;
  try {
    let data = JSON.parse(result);
    // O resultado de MCP também chega como lista de blocos de conteúdo: junta o texto e lê de novo.
    if (Array.isArray(data)) {
      const blocks = data as ({ type?: unknown; text?: unknown } | null)[];
      data = JSON.parse(blocks.map((b) => (b?.type === 'text' && typeof b.text === 'string' ? b.text : '')).join(''));
    }
    const page = data?.hangar_page;
    if (!page || typeof page.id !== 'string' || typeof page.title !== 'string') return null;
    return { id: page.id, title: page.title, height: typeof page.height === 'number' ? page.height : null, heights: page.heights ?? {} };
  } catch {
    return null;
  }
}

const clamp = (n: number) => Math.min(MAX, Math.max(MIN, Math.round(n)));

export function reservedHeight(ref: HtmlPageRef, width: number): number {
  const ws = Object.keys(ref.heights).map(Number).filter((w) => ref.heights[String(w)] > 0);
  if (!ws.length) return FALLBACK;
  const near = ws.reduce((a, b) => (Math.abs(b - width) < Math.abs(a - width) ? b : a));
  return clamp(ref.heights[String(near)]);
}

export function frameHeight(ref: HtmlPageRef, width: number, reported: number | null): number {
  const natural = reported ?? reservedHeight(ref, width);
  return clamp(ref.height != null ? Math.min(ref.height, natural) : natural);
}

export function pageFetchState(status: number | 'network'): PageFetchState {
  if (status === 404) return 'expired';
  if (typeof status === 'number' && status >= 200 && status < 300) return 'ready';
  return 'error';
}

export function pageUrls(base: string, session: string, id: string) {
  const root = `${base}/api/sessions/${encodeURIComponent(session)}/pages/${encodeURIComponent(id)}`;
  return {
    raw: `${root}?raw=1`,
    isolated: root,
    shot: (theme: 'dark' | 'light', width: number) => `${root}/shot?theme=${theme}&width=${Math.round(width)}`,
  };
}

const VARS = ['--foreground', '--muted-foreground', '--surface', '--border', '--accent', '--accent-foreground',
  '--danger', '--warning', '--success', '--code-background', '--chart-1', '--chart-2', '--chart-3', '--chart-4', '--font-sans', '--font-mono'];

// Lê do app (por nome da variável da página) o valor real do tema; vazio fica com o padrão injetado.
export function themeVariables(get: (name: string) => string): Record<string, string> {
  const out: Record<string, string> = { '--background': 'transparent' };
  for (const v of VARS) { const x = get(v).trim(); if (x) out[v] = x; }
  return out;
}
