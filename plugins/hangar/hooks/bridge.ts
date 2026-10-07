// A ponte da sessão, gravada só pelo input.ts e apagada quando o backend recusa a instância (409).
// Os outros arquivos leem SÓ daqui: um `claude -p` filho ou um segundo `claude` no mesmo pane
// herda o ambiente e não pode falar pela sessão.
export type Bridge = { url: string; token: string; sessao: string };

let atual: Bridge | null = null;
// O `idle` da largada sai antes de existir ponte: o `/pull` leva este valor para o backend.
let ultimoEstado: string | null = null;
const id =`${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

// Quem guarda algo só na memória do backend (a faixa dos mods) e reenvia quando a ponte aparece
// ou quando o backend diz que não tem (reiniciado, ele começa vazio).
const resendListeners = new Set<() => void>();

export function setBridge(b: Bridge): void {
  const novo = !atual;
  atual = b;
  if (novo) askResend();
}

export function askResend(): void {
  for (const cb of resendListeners) cb();
}

export function onResend(cb: () => void): void {
  resendListeners.add(cb);
}

export function clearBridge(): void {
  atual = null;
}

export function bridge(): Bridge | null {
  return atual;
}

// Ponte do clique do app pela superfície remota `desktop`, gravada só no `claude -p` pelo ui.ts. Ela não
// liga os outros hooks: lá o aviso e a cópia já chegam ao Hangar pelo canal da superfície (`ui_toast`,
// `ui_copy`), e sobra ao plugin só levar ao aparelho de quem clicou a URL que um mod abriria no servidor.
// Um `claude -p` filho que herde o ambiente não fala pela sessão: só o Hangar liga a superfície `desktop`.
let superficie: Bridge | null = null;

export function setSurfaceBridge(b: Bridge | null): void {
  superficie = b;
}

export function surfaceBridge(): Bridge | null {
  return superficie;
}

export function setLastState(estado: string): void {
  ultimoEstado = estado;
}

export function lastState(): string | null {
  return ultimoEstado;
}

export function instance(): string {
  return id;
}
