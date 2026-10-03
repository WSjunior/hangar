import type { EngineInterface, On } from "claude-code";
import { bridge } from "./bridge";

// A faixa redesenha a cada segundo enquanto um mod mostra relógio: o envio junta os quadros.
const SEND_DELAY_MS = 500;
// Um Raster cheio passa de 1 MB; acima disto a faixa sai vazia em vez de pesar o backend.
const MAX_TREE_CHARS = 256 * 1024;

let latest: string | null = null;
let sent: string | null = null;
let scheduled = false;

// Mesmo motivo do state.ts: `$` não atravessa import, então o envio é local.
async function flush($: EngineInterface) {
  scheduled = false;
  const body = latest;
  const p = bridge();
  if (body === null || body === sent || !p) return;
  sent = body;
  try {
    const r = await $.http.fetch(`${p.url}/ui`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: `{"sessao":${JSON.stringify(p.sessao)},"token":${JSON.stringify(p.token)},"above":${body}}`,
    });
    if (r.status !== 200) sent = null;
  } catch {
    // Backend fora do ar: o próximo redesenho tenta de novo.
    sent = null;
  }
}

/** Espelha no Hangar a faixa acima do prompt, a de TODOS os mods.
 *
 * `next(e)` devolve a árvore que os plugins abaixo deste e o engine desenharam; ela segue
 * intacta para a tela, e uma cópia vai ao backend. Só a superfície do terminal: com o app da
 * Anthropic aberto pelo Remote Control a mesma faixa também é pedida para `mobile`, e as duas
 * versões se alternariam no Hangar. */
export function registerUi(on: On) {
  on("ui.render", { component: "AbovePrompt" }, async ($, e, next) => {
    const tree = await next(e);
    if (e.surface === "terminal") {
      const empty = !tree || (tree as { type?: string }).type === "engine";
      const json = empty ? "null" : JSON.stringify(tree);
      latest = json.length > MAX_TREE_CHARS ? "null" : json;
      if (!scheduled && latest !== sent) {
        scheduled = true;
        $.clock.after(SEND_DELAY_MS, () => void flush($));
      }
    }
    return tree;
  });
}
