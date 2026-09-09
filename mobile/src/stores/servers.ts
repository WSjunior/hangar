import { create } from 'zustand';
import * as SecureStore from 'expo-secure-store';
import type { Server } from '@hangar/core';

const KEY = 'cp_servers_v1';

type Persisted = { servers: Server[]; activeId: string | null };

const hostLabel = (u: string) => {
  try {
    return new URL(u).hostname.split('.')[0] || u;
  } catch {
    return u;
  }
};

// Por que estes dois avisos existem: as duas formas de perder um servidor eram mudas. A gravação
// no keystore podia falhar (keystore invalidado depois de restaurar backup, disco cheio) com a
// tela seguindo em frente como se tivesse salvo — e no próximo boot o pareamento não estava lá; e
// um 401 apagava o servidor da lista sem uma palavra, deixando "nenhum servidor" no lugar de
// "o token desta máquina não vale mais".
export type AvisoServidores =
  | { tipo: 'persistencia' }
  | { tipo: 'token'; label: string };

export interface ServersState {
  servers: Server[];
  activeId: string | null;
  ready: boolean;
  aviso: AvisoServidores | null;
  load(): Promise<void>;
  add(s: Omit<Server, 'id' | 'label'> & { label?: string }): Server;
  remove(id: string): void;
  setActive(id: string): void;
  active(): Server | null;
  markInvalid(id: string): void;
  ensureActive(id: string): boolean;
  limparAviso(): void;
}

export const useServers = create<ServersState>((set, get) => ({
  servers: [],
  activeId: null,
  ready: false,
  aviso: null,
  async load() {
    try {
      const raw = await SecureStore.getItemAsync(KEY);
      const p: Persisted = raw ? JSON.parse(raw) : { servers: [], activeId: null };
      set({ ...p, ready: true });
    } catch {
      // JSON corrompido ou SecureStore falhou: reseta pra vazio e libera a tela
      set({ servers: [], activeId: null, ready: true });
    }
  },
  add({ baseUrl, token, label }) {
    const base = baseUrl.replace(/\/+$/, '');
    const existing = get().servers.find((s) => s.baseUrl === base);
    const s: Server = existing
      ? { ...existing, token }
      : { id: `s_${Date.now().toString(36)}`, label: label ?? hostLabel(base), baseUrl: base, token };
    const servers = existing ? get().servers.map((x) => (x.id === s.id ? s : x)) : [...get().servers, s];
    void persistir(set, { servers, activeId: s.id });
    set({ servers, activeId: s.id });
    return s;
  },
  remove(id) {
    const servers = get().servers.filter((s) => s.id !== id);
    const activeId = get().activeId === id ? (servers[0]?.id ?? null) : get().activeId;
    void persistir(set, { servers, activeId });
    set({ servers, activeId });
  },
  setActive(id) {
    void persistir(set, { servers: get().servers, activeId: id });
    set({ activeId: id });
  },
  active() {
    return get().servers.find((s) => s.id === get().activeId) ?? null;
  },
  markInvalid(id) {
    const label = get().servers.find((s) => s.id === id)?.label ?? '';
    get().remove(id);
    set({ aviso: { tipo: 'token', label } });
  },
  ensureActive(id) {
    if (!get().servers.some((s) => s.id === id)) return false;
    if (get().activeId !== id) get().setActive(id);
    return true;
  },
  limparAviso() {
    set({ aviso: null });
  },
}));

// A escrita continua sem `await` nos chamadores de propósito — a tela não deve travar esperando o
// keystore. O que mudou é que a falha vira estado, em vez de sumir num catch vazio: o estado em
// memória segue certo, a pessoa é avisada de que aquilo não sobreviveu a fechar o app, e não
// descobre isso só no próximo boot com a lista vazia.
function persistir(set: (p: Partial<ServersState>) => void, p: Persisted): Promise<void> {
  return SecureStore.setItemAsync(KEY, JSON.stringify(p)).catch(() => {
    set({ aviso: { tipo: 'persistencia' } });
  });
}
