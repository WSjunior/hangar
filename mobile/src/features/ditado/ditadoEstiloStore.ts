import { create } from 'zustand';
import { getConfig, patchConfig } from '@hangar/core';
import { ehEstilo } from '@hangar/core';
import type { EstiloDitado } from '@hangar/core';

const PADRAO: EstiloDitado = 'prosa';

let escritas = 0;
let carregando: Promise<void> | null = null;

interface DitadoEstiloState {
  valor: EstiloDitado;
  pronto: boolean;
  // A carga falhou: o valor em uso é o padrão, não o que está no servidor. Quem mostra o estilo
  // precisa saber disso — antes o erro sumia num catch vazio e a tela exibia `prosa` como se
  // fosse a escolha da pessoa.
  falhou: boolean;
  carregar: () => Promise<void>;
  revalidar: () => Promise<void>;
  trocar: (novo: EstiloDitado) => Promise<void>;
  _zerarParaTeste: () => void;
}

export const useDitadoEstiloStore = create<DitadoEstiloState>((set, get) => ({
  valor: PADRAO,
  pronto: false,
  falhou: false,

  carregar: async () => {
    if (get().pronto) return;
    if (carregando) return carregando;
    const escritasNoInicio = escritas;
    carregando = getConfig()
      .then((cfg) => {
        const v = cfg.campos?.ditado_estilo?.valor;
        if (ehEstilo(v) && escritas === escritasNoInicio) {
          set({ valor: v });
        }
        set({ pronto: true, falhou: false });
      })
      .catch(() => {
        // `pronto` mesmo assim: sem ele a tela fica esperando para sempre. O que muda é que
        // `falhou` diz que o valor exibido é o padrão, não o do servidor.
        set({ pronto: true, falhou: true });
      })
      .finally(() => {
        carregando = null;
      });
    return carregando;
  },

  revalidar: async () => {
    set({ pronto: false });
    return get().carregar();
  },

  trocar: async (novo: EstiloDitado) => {
    const antes = get().valor;
    const minha = ++escritas;
    set({ valor: novo });
    try {
      await patchConfig({ ditado_estilo: novo });
      set({ pronto: true, falhou: false });
    } catch (e) {
      if (escritas === minha) {
        set({ valor: antes });
      }
      throw e;
    }
  },

  _zerarParaTeste: () => {
    escritas = 0;
    carregando = null;
    set({ valor: PADRAO, pronto: false, falhou: false });
  },
}));
