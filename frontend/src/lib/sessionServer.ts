import { getContext, onDestroy, setContext } from 'svelte';
import { listAllServers, type Server } from './auth';

// Servidor da sessão aberta no chat, fixado na entrada. O chat e as folhas dele chamam a API por
// ele, nunca pelo ativo do momento: o ativo muda por baixo de um chat aberto (overlay, withServer,
// outra aba da mesma origem) e a chamada ia parar numa máquina que não tem a sessão.
const KEY = Symbol('sessionServer');
const ID_KEY = Symbol('sessionServerId');

export type SessionServer = () => Server | undefined;

// Resolve na hora da chamada: token ou endereço atualizados valem já. Desligada inclusive, como o
// ativo (`getActiveId` também a enxerga). Removida com o chat aberto, segue no último endereço
// conhecido DAQUELA máquina: cair no ativo mandaria a chamada a outra máquina sem ninguém ver, e
// um erro aqui quebrava o chat, que lê o servidor em `$derived`, `$effect` e no reconectar do SSE.
export function sessionServerFor(id: string): SessionServer {
  let last = id ? listAllServers().find((x) => x.id === id) : undefined;
  return () => {
    if (!id) return undefined;
    last = listAllServers().find((x) => x.id === id) ?? last;
    return last;
  };
}

// Máquinas com chat aberto: como o ativo, não entram em espera por uma falha de rede numa chamada
// lateral, senão o envio seguinte seria recusado sem nem tentar.
const openChats = new Map<string, number>();

export function hasOpenChat(id: string): boolean {
  return (openChats.get(id) ?? 0) > 0;
}

export function provideSessionServer(id: string): SessionServer {
  const ref = sessionServerFor(id);
  setContext(KEY, ref);
  setContext(ID_KEY, id);
  if (id) {
    openChats.set(id, (openChats.get(id) ?? 0) + 1);
    onDestroy(() => {
      const n = (openChats.get(id) ?? 1) - 1;
      if (n > 0) openChats.set(id, n); else openChats.delete(id);
    });
  }
  return ref;
}

/** Fora de um chat (lista, configurações) não há dono fixado: as chamadas seguem o ativo. */
export function useSessionServer(): SessionServer {
  return getContext<SessionServer | undefined>(KEY) ?? (() => undefined);
}

/** Id fixado pelo chat de fora: o chat aninhado (sessão do par) é da mesma máquina, não do ativo. */
export function parentSessionServerId(): string | undefined {
  return getContext<string | undefined>(ID_KEY) || undefined;
}
