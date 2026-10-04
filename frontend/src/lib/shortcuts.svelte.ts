// DONO da lista de atalhos da fileira, por servidor. Mora fora do Chat porque três superfícies
// consomem a mesma lista (fileira do painel, "⋯" da NavBar, command palette) e o editor da
// config grava nela — uma busca por montagem de Chat viraria três GETs e três verdades.
import {
  defaultShortcuts, getConfig, getConfigForServer, getProjectShortcuts, patchConfig, patchConfigForServer,
  putProjectShortcuts, resolveShortcuts, serializeShortcuts,
  type ProjectShortcut, type ProjectShortcuts, type ScopedShortcut, type Shortcut,
  type ShortcutSendText, type ShortcutShell,
} from '@hangar/core';
import { getActiveId, listServers, type Server } from './auth';
import * as m from '../paraglide/messages';

const lists = $state<Record<string, Shortcut[]>>({});
// Promessa, não flag: quem chega com a busca em voo tem de esperar a lista real, senão o editor
// abre com o conjunto nativo e salvar apagaria a config da pessoa.
const inFlight = new Map<string, Promise<void>>();

function keyFor(serverId?: string | null): string {
  return serverId || getActiveId() || '';
}

/** Lista renderizável do servidor (default nativo enquanto não carregou). Pura: quem monta
 * chama `loadShortcuts` num $effect/onMount — side-effect dentro de $derived travaria o Svelte. */
export function shortcutsFor(serverId?: string | null): Shortcut[] {
  return lists[keyFor(serverId)] ?? defaultShortcuts();
}

/** Falha rejeita: enquanto isso a fileira segue no default nativo, e quem chama decide como
 * mostrar o erro. Não cacheia a falha — a próxima chamada tenta de novo. */
export function loadShortcuts(serverId?: string | null): Promise<void> {
  const k = keyFor(serverId);
  if (!k || k in lists) return Promise.resolve();
  const pending = inFlight.get(k);
  if (pending) return pending;
  const load = (async () => {
    const s = serverId && serverId !== getActiveId()
      ? listServers().find((x) => x.id === serverId)
      : null;
    const c = s ? await getConfigForServer(s) : await getConfig();
    lists[k] = resolveShortcuts(String(c.campos.shortcuts?.valor ?? ''));
  })().finally(() => inFlight.delete(k));
  inFlight.set(k, load);
  return load;
}

/** Grava a lista no servidor e atualiza o cache. `null` = remover o override (volta ao nativo). */
export async function saveShortcuts(list: Shortcut[] | null, serverId?: string | null): Promise<void> {
  const k = keyFor(serverId);
  const s = serverId && serverId !== getActiveId()
    ? listServers().find((x) => x.id === serverId)
    : null;
  const value = list === null ? null : serializeShortcuts(list);
  if (s) await patchConfigForServer(s, { shortcuts: value });
  else await patchConfig({ shortcuts: value });
  lists[k] = list === null ? defaultShortcuts() : list;
}

/** Relê do servidor, descartando o cache: depois de uma importação, a lista gravada mudou lá. */
export function reloadShortcuts(serverId?: string | null): Promise<void> {
  delete lists[keyFor(serverId)];
  return loadShortcuts(serverId);
}

// ── Atalhos do projeto da sessão, por servidor+sessão. Sem `server`, as rotas falam com o
// servidor ATIVO e a chave usa ele. Carregar e falhar nunca mexem na lista global. ─────────────
const projects = $state<Record<string, ProjectShortcuts>>({});
const projectErrors = $state<Record<string, string>>({});
const projectInFlight = new Map<string, Promise<void>>();

function projectKey(session: string, server?: Server | null): string {
  return `${server?.id ?? getActiveId() ?? ''}::${session}`;
}

export function projectShortcutsFor(session: string, server?: Server | null): ProjectShortcuts | null {
  return projects[projectKey(session, server)] ?? null;
}

/** Mensagem (já traduzida pelo `code`) da última leitura que falhou; vazio = sem erro. */
export function projectShortcutsError(session: string, server?: Server | null): string {
  return projectErrors[projectKey(session, server)] ?? '';
}

/** Sempre relê: o arquivo é do projeto, e outra sessão/cliente pode ter gravado nele. A lista
 * anterior fica na tela até a nova chegar. Falha rejeita e fica guardada pra fileira mostrar. */
export function loadProjectShortcuts(session: string, server?: Server | null): Promise<void> {
  const k = projectKey(session, server);
  const pending = projectInFlight.get(k);
  if (pending) return pending;
  const load = (async () => {
    try {
      projects[k] = await getProjectShortcuts(session, server);
      delete projectErrors[k];
    } catch (err) {
      // Sem `status` o pedido nem chegou ao servidor, e a mensagem é a do navegador, em inglês.
      projectErrors[k] = err instanceof Error && 'status' in err ? err.message : m.falha_conexao();
      throw err;
    }
  })().finally(() => projectInFlight.delete(k));
  projectInFlight.set(k, load);
  return load;
}

/** Grava a lista inteira do projeto e guarda como o servidor devolveu. */
export async function saveProjectShortcuts(session: string, items: ProjectShortcut[], server?: Server | null): Promise<ProjectShortcuts> {
  const k = projectKey(session, server);
  const saved = await putProjectShortcuts(session, items, server);
  projects[k] = saved;
  delete projectErrors[k];
  return saved;
}

/** Item da junção global+projeto que vira bloco (os internos ficam na fileira do topo). */
export type CustomScoped = ScopedShortcut & { shortcut: ShortcutSendText | ShortcutShell };

export function customOf(list: ScopedShortcut[]): CustomScoped[] {
  return list.filter((s): s is CustomScoped => s.shortcut.type !== 'internal');
}
