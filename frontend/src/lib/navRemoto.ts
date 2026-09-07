// Endereço do acesso remoto ao navegador da sessão. Mesmo molde do `termUrlForServer` (term.ts:27),
// inclusive o fallback pra `location.origin`: o servidor de mesma origem guarda baseUrl vazio e
// `new WebSocket('/api/...')` sozinho levanta SyntaxError.
import { getBaseUrl, getToken } from './auth';

export function navUrl(name: string): string {
  const base = (getBaseUrl() || location.origin).replace(/^http/, 'ws');
  const qs = new URLSearchParams({ token: getToken() || '' });
  // `nav-remoto`, não `nav`: o `POST/DELETE /nav` já existe (marcador de página pendente do
  // `hangar-preview open`) e um WebSocket com o mesmo caminho seria só confusão pra quem lê o log.
  return `${base}/api/sessions/${encodeURIComponent(name)}/nav-remoto?${qs}`;
}

export type QuadroNav = { d: string; w: number; h: number; lento?: boolean };
