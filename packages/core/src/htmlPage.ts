// Página publicada pelo agente (tool html_render do MCP hangar), desenhada no lugar da chamada.
// `ownTheme`: a página leva as próprias cores e o app não lhe passa o tema.
// `url`: site de verdade (modo URL); não há HTML para buscar, o app abre o endereço.
export type HtmlPageRef = { id: string; title: string; height: number | null; heights: Record<string, number>; ownTheme: boolean; url?: string };
export type PageFetchState = 'loading' | 'ready' | 'error' | 'expired';

const MIN = 80;
const MAX = 2000;
const FALLBACK = 240;
// Claude e o modo código do Codex gravam `mcp__hangar__html_render`; outros clientes MCP separam o
// servidor da tool por `.` ou `/`. Mesma regra do `is_page_call` do nativo.
const NAME = /^(?:mcp__)?hangar(?:__|\.|\/)html_render$/;

export function isHtmlRenderTool(name: string | null | undefined): boolean {
  return !!name && NAME.test(name);
}

type Block = { type?: unknown; text?: unknown } | null;
const joinText = (blocks: Block[]) => blocks.map((b) => (b?.type === 'text' && typeof b.text === 'string' ? b.text : '')).join('');

export function htmlPageFromResult(toolName: string | null | undefined, result: string | null | undefined): HtmlPageRef | null {
  if (!isHtmlRenderTool(toolName) || !result) return null;
  try {
    let data = JSON.parse(result);
    // O resultado de MCP também chega como lista de blocos de conteúdo: junta o texto e lê de novo.
    if (Array.isArray(data)) data = JSON.parse(joinText(data as Block[]));
    // O Codex grava o CallToolResult inteiro: o estruturado vence, senão o texto dos blocos.
    else if (data && Array.isArray(data.content) && !data.hangar_page) {
      data = data.structuredContent?.hangar_page ? data.structuredContent : JSON.parse(joinText(data.content as Block[]));
    }
    const page = data?.hangar_page;
    if (!page || typeof page.id !== 'string' || typeof page.title !== 'string') return null;
    const ref: HtmlPageRef = { id: page.id, title: page.title, height: typeof page.height === 'number' ? page.height : null,
      heights: page.heights ?? {}, ownTheme: page.own_theme === true };
    // Só http(s): o app abre esse endereço direto.
    if (typeof page.url === 'string' && /^https?:\/\//i.test(page.url)) ref.url = page.url;
    return ref;
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

// `code` é o `detail.code` do corpo JSON da resposta: só o servidor de páginas diz `erro_pagina_expirou`;
// um 404 genérico (convidado, rota ausente no Python) é erro, não página vencida.
export function pageFetchState(status: number | 'network', code?: string | null): PageFetchState {
  if (status === 404 && code === 'erro_pagina_expirou') return 'expired';
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
