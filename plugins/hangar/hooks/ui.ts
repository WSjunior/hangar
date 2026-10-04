import type { EngineInterface, On } from "claude-code";
import { bridge } from "./bridge";
import { bandBody, type PaneEntry } from "./uiPayload";

// A faixa redesenha a cada segundo enquanto um mod mostra relógio: o envio junta os quadros.
const SEND_DELAY_MS = 500;
// Um Raster cheio passa de 1 MB; acima disto saem os painéis e depois a faixa.
const MAX_BODY_CHARS = 256 * 1024;

let above: unknown = null;
let columns: number | null = null;
const panes = new Map<string, PaneEntry>();
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
      body: `{"sessao":${JSON.stringify(p.sessao)},"token":${JSON.stringify(p.token)},${body}}`,
    });
    if (r.status !== 200) sent = null;
  } catch {
    // Backend fora do ar: o próximo redesenho tenta de novo.
    sent = null;
  }
}

function schedule($: EngineInterface) {
  latest = bandBody(above, columns, [...panes.values()], MAX_BODY_CHARS);
  if (!scheduled && latest !== sent) {
    scheduled = true;
    $.clock.after(SEND_DELAY_MS, () => void flush($));
  }
}

// Clique e cópia confirmam ao backend o clique que o app pediu; sem ponte, ninguém pediu.
async function tell($: EngineInterface, path: "pressed" | "copied", fields: Record<string, unknown>) {
  const p = bridge();
  if (!p) return;
  try {
    await $.http.fetch(`${p.url}/${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessao: p.sessao, token: p.token, ...fields }),
    });
  } catch {
    // O backend responde ao app por tempo esgotado.
  }
}

/** Espelha no Hangar a faixa acima do prompt e os painéis, os de TODOS os mods.
 *
 * `next(e)` devolve a árvore que os plugins abaixo deste e o engine desenharam; ela segue
 * intacta para a tela, e uma cópia vai ao backend. Só a superfície do terminal: com o app da
 * Anthropic aberto pelo Remote Control a mesma faixa também é pedida para `mobile`, e as duas
 * versões se alternariam no Hangar. */
export function registerUi(on: On) {
  on("ui.render", { component: "AbovePrompt" }, async ($, e, next) => {
    const tree = await next(e);
    if (e.surface === "terminal") {
      above = tree;
      // As 5 colunas do `[-]` voltam: é a largura da coluna da conversa, o corte da prévia.
      columns = e.props.bodyColumns + 5;
      schedule($);
    }
    return tree;
  });

  on("ui.render", { component: "Pane" }, async ($, e, next) => {
    const tree = await next(e);
    if (e.surface === "terminal") {
      panes.set(e.requestId, {
        id: e.requestId, title: e.props.title, placement: e.props.placement, columns: e.props.bodyColumns, tree,
      });
      schedule($);
    }
    return tree;
  });

  on("ui.close", async ($, e, next) => {
    const r = await next(e);
    if (!(r as { deny?: unknown } | undefined)?.deny && panes.delete(e.id)) schedule($);
    return r;
  });

  on("ui.press", async ($, e, next) => {
    const r = await next(e);
    if (e.surface === "terminal") void tell($, "pressed", { requestId: e.requestId, element: e.element });
    return r;
  });

  on("ui.copy", async ($, e, next) => {
    const r = await next(e);
    if (r.value?.isCopied) void tell($, "copied", { text: e.text });
    return r;
  });
}
