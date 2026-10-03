// Fileira de atalhos configurável do painel de sessão. A lista mora no servidor
// (runtime_config, campo `shortcuts`: JSON numa string; vazio = conjunto nativo) e as
// superfícies (fileira desktop, "⋯" da NavBar, command palette) consomem o resolve daqui.

/** Ações internas: os botões nativos de hoje. Rótulo/ícone vêm do front (i18n), não da config. */
/** `externo`: terminal do sistema anexado à sessão; só o app nativo o executa, e só com servidor local. */
export type ShortcutInternalAction = 'terminal' | 'modo' | 'navegador' | 'anexos' | 'rodar' | 'externo';

export interface ShortcutInternal {
  id: string;
  type: 'internal';
  action: ShortcutInternalAction;
  confirm?: boolean;
}

export interface ShortcutSendText {
  id: string;
  type: 'send_text';
  label: string;
  icon?: string;          // "emoji:🚀" ou "glifo:bolt" (chave do mapa do ShortcutIcon)
  text: string;           // texto completo a enviar (ex.: "/relatorio-pm" ou um prompt)
  send_direct?: boolean;  // ausente = true; false pré-preenche o composer
  confirm?: boolean;
}

export interface ShortcutShell {
  id: string;
  type: 'shell';
  label: string;
  icon?: string;
  command: string;        // roda na máquina do servidor, num terminal escondido; cwd da sessão (ou home, No Hangar)
  pasta?: string;         // absoluta ou relativa à raiz da cópia da sessão; ausente = cwd da sessão
  // 'hangar' = uma cópia só no servidor, fora de qualquer sessão; ausente = da sessão que clicou.
  runs_in?: 'session' | 'hangar';
  hangar_home?: boolean;  // No Hangar: ausente = roda na home
  answer_in_app?: boolean; // ausente = a pergunta do terminal aparece no app
  confirm?: boolean;
}

export type Shortcut = ShortcutInternal | ShortcutSendText | ShortcutShell;

const INTERNAL_ACTIONS: ShortcutInternalAction[] = [
  'terminal', 'modo', 'navegador', 'anexos', 'rodar', 'externo',
];

/** A fileira de hoje, na ordem de hoje. É o que vale com config vazia. */
export function defaultShortcuts(): Shortcut[] {
  return INTERNAL_ACTIONS.map((action) => ({ id: action, type: 'internal', action }));
}

function isValid(item: unknown): item is Shortcut {
  if (typeof item !== 'object' || item === null) return false;
  const o = item as Record<string, unknown>;
  if (typeof o.id !== 'string' || !o.id.trim()) return false;
  // Opcionais com tipo errado derrubariam quem lê (o ícone chama `.startsWith`).
  if (o.icon !== undefined && typeof o.icon !== 'string') return false;
  if (o.confirm !== undefined && typeof o.confirm !== 'boolean') return false;
  if (o.send_direct !== undefined && typeof o.send_direct !== 'boolean') return false;
  if (o.type === 'internal') {
    return INTERNAL_ACTIONS.includes(o.action as ShortcutInternalAction);
  }
  if (o.type === 'send_text') {
    return typeof o.label === 'string' && !!o.label.trim()
      && typeof o.text === 'string' && !!o.text.trim();
  }
  if (o.type === 'shell') {
    return typeof o.label === 'string' && !!o.label.trim()
      && typeof o.command === 'string' && !!o.command.trim()
      && (o.pasta === undefined || (typeof o.pasta === 'string' && !!o.pasta.trim()))
      && (o.runs_in === undefined || o.runs_in === 'session' || o.runs_in === 'hangar')
      && (o.hangar_home === undefined || typeof o.hangar_home === 'boolean')
      && (o.answer_in_app === undefined || typeof o.answer_in_app === 'boolean');
  }
  return false;
}

/** Resolve a string da config numa lista renderizável. NUNCA lança: config vazia, JSON quebrado
 * ou shape estranho caem no conjunto nativo — o backend valida na gravação, então lixo aqui é
 * config de versão antiga, e a fileira não pode sumir por causa dela. Item individual inválido
 * é descartado (não derruba a lista inteira). */
export function resolveShortcuts(raw: string | null | undefined): Shortcut[] {
  if (!raw || !raw.trim()) return defaultShortcuts();
  let data: unknown;
  try {
    data = JSON.parse(raw);
  } catch {
    return defaultShortcuts();
  }
  if (!Array.isArray(data)) return defaultShortcuts();
  // id repetido quebra o {#each} com chave: fica o primeiro.
  const seen = new Set<string>();
  return data.filter(isValid).filter((s) => !seen.has(s.id) && !!seen.add(s.id));
}

export function serializeShortcuts(list: Shortcut[]): string {
  return JSON.stringify(list);
}

/** Atalho do projeto: só `send_text` e `shell` (os internos são do servidor inteiro). */
export type ProjectShortcut = ShortcutSendText | ShortcutShell;

/** Resposta de GET/PUT `/api/sessions/{name}/project-shortcuts`. `key` = repositório git (ou a
 * pasta, fora de git); `root` = raiz da cópia da sessão, contra a qual `pasta` relativa resolve. */
export interface ProjectShortcuts {
  key: string;
  name: string;
  root: string;
  items: ProjectShortcut[];
}

export type ShortcutScope = 'global' | 'project';

export interface ScopedShortcut {
  shortcut: Shortcut;
  scope: ShortcutScope;
  key: string;  // `${scope}:${id}`: o mesmo id nas duas listas não colide no {#each} com chave
}

/** Globais e depois os do projeto, cada um com o escopo. Item do projeto inválido ou interno
 * (arquivo editado à mão) cai fora, e id repetido dentro do projeto fica só o primeiro. */
export function mergeProjectShortcuts(
  global: Shortcut[], project: unknown[] | null | undefined,
): ScopedShortcut[] {
  const seen = new Set<string>();
  const own = (project ?? []).filter(
    (s): s is ProjectShortcut => isValid(s) && s.type !== 'internal' && !seen.has(s.id) && !!seen.add(s.id),
  );
  const tag = (scope: ShortcutScope) => (shortcut: Shortcut): ScopedShortcut =>
    ({ shortcut, scope, key: `${scope}:${shortcut.id}` });
  return [...global.map(tag('global')), ...own.map(tag('project'))];
}

/** true quando o atalho envia direto (padrão do send_text; flag desligada = pré-preencher). */
export function sendsDirect(s: ShortcutSendText): boolean {
  return s.send_direct !== false;
}

// `boolean`, não type guard: com guard, o `if (runsInHangar(s)) return;` estreita `s` pra `never`
// no resto da função.
export function runsInHangar(s: ShortcutSendText | ShortcutShell): boolean {
  return s.type === 'shell' && s.runs_in === 'hangar';
}

/** Identidade da cópia No Hangar: global = `global:<id>`; do projeto = `project:<chave do repo>:<id>`
 * (o mesmo id em repositórios diferentes não pode dividir uma cópia). */
export function hangarKeyOf(scope: ShortcutScope, id: string, projectKey?: string): string {
  return scope === 'project' ? `project:${projectKey ?? ''}:${id}` : `global:${id}`;
}
export function hangarHome(s: ShortcutShell): boolean { return s.hangar_home !== false; }
export function answersInApp(s: ShortcutShell): boolean { return s.answer_in_app !== false; }

/** Credencial que a importação deixou em branco (`⟦SEGREDO:<nome>⟧` no comando ou texto): o
 * atalho fica salvo mas não roda até alguém preencher. Devolve o nome, ou null. */
export function shortcutMissingSecret(s: ShortcutSendText | ShortcutShell): string | null {
  const m = /⟦SEGREDO:([A-Za-z0-9_.-]+)⟧/.exec(s.type === 'shell' ? s.command : s.text);
  return m ? m[1] : null;
}
