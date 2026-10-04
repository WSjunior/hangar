import type { EngineInterface, On } from "claude-code";
import { bridge } from "./bridge";
import { openerUrl } from "./uiIntercept";
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

// Janela do clique que o app pediu: abrir URL e copiar vão para o aparelho de quem clicou. Não acaba
// no fim do `next`: o `onPress` do mod costuma disparar a cópia sem `await`. Clique feito no próprio
// terminal fecha a janela na hora.
const APP_PRESS_MS = 1500;
let appUntil = 0;

async function inAppPress($: EngineInterface): Promise<boolean> {
  return appUntil > 0 && (await $.clock.now()) < appUntil;
}

// Clique, cópia e abertura confirmam ao backend o clique que o app pediu; sem ponte, ninguém pediu.
async function tell($: EngineInterface, path: "pressed" | "copied" | "opened", fields: Record<string, unknown>) {
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

// O press que começou no terminal é o clique que o app pediu? O backend responde sim uma vez só.
async function fromApp($: EngineInterface, requestId: string, element: string): Promise<boolean> {
  const p = bridge();
  if (!p) return false;
  try {
    const r = await $.http.fetch(`${p.url}/press-start`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessao: p.sessao, token: p.token, requestId, element }),
    });
    return r.status === 200 && (JSON.parse(r.text) as { fromApp?: boolean }).fromApp === true;
  } catch {
    return false;
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
    if (e.surface === "terminal") {
      const app = await fromApp($, e.requestId, e.element);
      appUntil = app ? (await $.clock.now()) + APP_PRESS_MS : 0;
    }
    try {
      return await next(e);
    } finally {
      if (e.surface === "terminal") void tell($, "pressed", { requestId: e.requestId, element: e.element });
    }
  });

  on("ui.copy", async ($, e, next) => {
    if (!(await inAppPress($))) return next(e);
    await tell($, "copied", { text: e.text });
    return { value: { isCopied: true } };
  });

  on("process.run", async ($, e, next) => {
    const url = (await inAppPress($)) ? openerUrl(e.argv) : null;
    if (!url) return next(e);
    await tell($, "opened", { url });
    return { value: { exitCode: 0, stdout: "", stderr: "", isStdoutTruncated: false, isStderrTruncated: false } };
  });
}
