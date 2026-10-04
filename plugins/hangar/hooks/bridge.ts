// A ponte da sessão, gravada só pelo input.ts e apagada quando o backend recusa a instância (409).
// Os outros arquivos leem SÓ daqui: um `claude -p` filho ou um segundo `claude` no mesmo pane
// herda o ambiente e não pode falar pela sessão.
export type Bridge = { url: string; token: string; sessao: string };

let atual: Bridge | null = null;
// O `idle` da largada sai antes de existir ponte: o `/pull` leva este valor para o backend.
let ultimoEstado: string | null = null;
const id =`${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

// Quem guarda algo só na memória do backend (a faixa dos mods) e precisa reenviar quando a ponte
// aparece ou volta depois de o backend não responder (reiniciado, ele começa vazio).
const readyListeners = new Set<() => void>();
let reachable = false;

export function setBridge(b: Bridge): void {
  const back = !atual || !reachable;
  atual = b;
  reachable = true;
  if (back) for (const cb of readyListeners) cb();
}

/** O backend não respondeu: a próxima ponte aceita pode ser de um backend que perdeu o que guardava. */
export function markUnreachable(): void {
  reachable = false;
}

export function onBridgeReady(cb: () => void): void {
  readyListeners.add(cb);
}

export function clearBridge(): void {
  atual = null;
}

export function bridge(): Bridge | null {
  return atual;
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
