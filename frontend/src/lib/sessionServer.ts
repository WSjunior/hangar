import { getContext, onDestroy, setContext } from 'svelte';
import * as m from '../paraglide/messages';
import { listServers, type Server } from './auth';

// Servidor da sessão aberta no chat, fixado na entrada. O chat e as folhas dele chamam a API por
// ele, nunca pelo ativo do momento: o ativo muda por baixo de um chat aberto (overlay, withServer,
// outra aba da mesma origem) e a chamada ia parar numa máquina que não tem a sessão.
const KEY = Symbol('sessionServer');

export type SessionServer = () => Server | undefined;

// Resolve na hora da chamada (não guarda o objeto): token ou endereço atualizados valem já.
// Dono fixado que saiu da lista (removido ou desligado) é erro: cair no ativo mandaria a chamada
// a outra máquina sem ninguém ver.
export function sessionServerFor(id: string): SessionServer {
  return () => {
    if (!id) return undefined;
    const s = listServers().find((x) => x.id === id);
    if (!s) throw new Error(m.servidor_nao_existe());
    return s;
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
