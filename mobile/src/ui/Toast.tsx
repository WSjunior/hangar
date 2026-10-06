import { toast as sonner, Toaster } from 'sonner-native';

const MOD_TEXT_MAX = 300;
const MOD_VISIBLE_MAX = 4;
// Avisos de mod na tela, do mais antigo ao mais novo: um mod insistente não empilha a tela inteira.
const modVisible: (string | number)[] = [];

function modGone(id: string | number): void {
  const i = modVisible.indexOf(id);
  if (i >= 0) modVisible.splice(i, 1);
}

export const toast = {
  ok: (msg: string) => sonner.success(msg),
  erro: (msg: string) => sonner.error(msg),
  aviso: (msg: string) => sonner(msg, { duration: 12_000 }),
  /** Aviso de um mod do Claude Code: o nome do mod é o título, como na caixa do terminal. */
  mod: (msg: string, plugin: string, ms: number) => {
    while (modVisible.length >= MOD_VISIBLE_MAX) sonner.dismiss(modVisible.shift());
    const text = msg.length > MOD_TEXT_MAX ? `${msg.slice(0, MOD_TEXT_MAX)}…` : msg;
    const opts = { duration: ms, onDismiss: modGone, onAutoClose: modGone };
    const id = plugin ? sonner(plugin, { ...opts, description: text }) : sonner(text, opts);
    modVisible.push(id);
    return id;
  },
};
export { Toaster };
