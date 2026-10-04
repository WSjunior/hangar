import { toast as sonner, Toaster } from 'sonner-native';

export const toast = {
  ok: (msg: string) => sonner.success(msg),
  erro: (msg: string) => sonner.error(msg),
  /** Aviso de um mod do Claude Code: o nome do mod é o título, como na caixa do terminal. */
  mod: (msg: string, plugin: string, ms: number) =>
    plugin ? sonner(plugin, { description: msg, duration: ms }) : sonner(msg, { duration: ms }),
};
export { Toaster };
