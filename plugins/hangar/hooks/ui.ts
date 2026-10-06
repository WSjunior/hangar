import type { EngineInterface, On } from "claude-code";
import { type Bridge, bridge, onResend, setSurfaceBridge, surfaceBridge } from "./bridge";
import { openerUrl } from "./uiIntercept";
import { bandBody, type PaneEntry } from "./uiPayload";

// A faixa redesenha a cada segundo enquanto um mod mostra relógio: o envio junta os quadros.
const SEND_DELAY_MS = 500;
// Um Raster cheio passa de 1 MB; acima disto saem os painéis e depois a faixa.
const MAX_BODY_CHARS = 256 * 1024;

let above: unknown = null;
let columns: number | null = null;
const panes = new Map<string, PaneEntry>();
let sent: string | null = null;
let scheduled = false;
// Reenvio pedido quando a ponte volta, fora de qualquer hook: o engine não deixa guardar o `$`
// numa variável, só usá-lo num closure, como nos timers.
let resend: (() => void) | null = null;

// JSON de objeto sem as chaves de fora, para juntar à ponte no corpo do POST.
const fields = (o: Record<string, unknown>) => JSON.stringify(o).slice(1, -1);

// Mesmo motivo do state.ts: `$` não atravessa import, então o POST é local. `ponte`: a do terminal, ou a
// da superfície `desktop` no clique do app dentro do `claude -p`. Sem ponte, null.
async function post($: EngineInterface, path: string, extra: string, ponte: Bridge | null = bridge()): Promise<{ status: number; text: string } | null> {
  if (!ponte) return null;
  try {
    return await $.http.fetch(`${ponte.url}/${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: `{"sessao":${JSON.stringify(ponte.sessao)},"token":${JSON.stringify(ponte.token)},${extra}}`,
    });
  } catch {
    return null;
  }
}

// Serializa só aqui, no máximo a cada SEND_DELAY_MS: o hook de render só marca que mudou. Sem
// ponte ou com recusa, a faixa sai de novo quando a ponte aparece ou o backend diz que não a tem.
async function flush($: EngineInterface) {
  scheduled = false;
  const body = bandBody(above, columns, [...panes.values()], MAX_BODY_CHARS);
  if (body === sent || !bridge()) return;
  sent = body;
  if ((await post($, "ui", body))?.status !== 200) sent = null;
}

function schedule($: EngineInterface) {
  resend = () => {
    sent = null;
    schedule($);
  };
  if (!scheduled) {
    scheduled = true;
    $.clock.after(SEND_DELAY_MS, () => void flush($));
  }
}

onResend(() => resend?.());

// Janela do clique que o app pediu: abrir URL e copiar vão para o aparelho de quem clicou. Não acaba
// no fim do `next`: o `onPress` do mod costuma disparar a cópia sem `await`. Só vale para chamadas do
// mod dono do botão, e clique feito no próprio terminal fecha a janela na hora.
const APP_PRESS_MS = 1500;
// `surface`: só no terminal a cópia vai pela ponte; na superfície `desktop` o Hangar já recebe o
// `ui_copy` do engine. `ponte`: a do aparelho do clique (a do terminal ou a da superfície).
type AppPress = { until: number; plugin: string; attempt: string; surface: string; ponte: Bridge };
let appPress: AppPress | null = null;

// O clique do app a que esta chamada pertence, ou null para seguir onde o mod roda.
async function appAttempt($: EngineInterface, origin: string | undefined): Promise<AppPress | null> {
  if (!appPress || origin !== appPress.plugin) return null;
  return (await $.clock.now()) < appPress.until ? appPress : null;
}

// Quem fez a chamada do `$` que está passando por este hook.
function originOf(next: unknown): string | undefined {
  return (next as { origin?: { plugin?: string } }).origin?.plugin;
}

// Clique, cópia e abertura confirmam ao backend o clique que o app pediu. Devolve se o backend
// aceitou: recusado, a cópia ou a abertura acontece onde o mod roda.
async function tell($: EngineInterface, path: "pressed" | "copied" | "opened", o: Record<string, unknown>, ponte: Bridge | null = bridge()): Promise<boolean> {
  return (await post($, path, fields(o), ponte))?.status === 200;
}

// O press que começou aqui é o clique que o app pediu? O backend responde com a tentativa, uma vez só.
async function fromApp($: EngineInterface, requestId: string, element: string, ponte: Bridge | null = bridge()): Promise<string | null> {
  const r = await post($, "press-start", fields({ requestId, element }), ponte);
  if (r?.status !== 200) return null;
  const { attempt } = JSON.parse(r.text) as { attempt?: string | null };
  return typeof attempt === "string" && attempt ? attempt : null;
}

/** Espelha no Hangar a faixa acima do prompt, os painéis e os avisos, os de TODOS os mods.
 *
 * `next(e)` devolve a árvore que os plugins abaixo deste e o engine desenharam; ela segue
 * intacta para a tela, e uma cópia vai ao backend. Só a superfície do terminal: com o app da
 * Anthropic aberto pelo Remote Control a mesma faixa também é pedida para `mobile`, e as duas
 * versões se alternariam no Hangar. */
export function registerUi(on: On) {
  // Sem terminal (`claude -p`) o Hangar é a superfície `desktop` dos mods. Aqui só nasce a ponte do
  // clique; a do input.ts fica nula e os outros hooks seguem calados nessa sessão. Com matcher, como o
  // `{ isInteractive: true }` do input.ts: o state.ts tem o único `session.start` sem matcher.
  on("session.start", { isInteractive: false }, async ($, e, next) => {
    const url = await $.env.get("HANGAR_PLUGIN_URL");
    const token = await $.env.get("HANGAR_PLUGIN_TOKEN");
    const sessao = await $.env.get("CP_SESSION_NAME");
    setSurfaceBridge(url && token && sessao ? { url, token, sessao } : null);
    return next(e);
  });

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

  // Reaberto com as mesmas props, o painel volta com o desenho que o engine guardou, sem `ui.render`:
  // sem pedir o desenho de novo, o Hangar ficaria sem o painel que o terminal mostra.
  on("ui.open", async ($, e, next) => {
    const r = await next(e);
    if (!panes.has(e.id)) $.ui.invalidate("ui.render");
    return r;
  });

  on("ui.close", async ($, e, next) => {
    const r = await next(e);
    if (!(r as { deny?: unknown } | undefined)?.deny && panes.delete(e.id)) schedule($);
    return r;
  });

  // O aviso só é desenhado no terminal e não entra no transcript: sem a cópia, quem acompanha a
  // sessão pelo app não o vê. Leva o nome do mod que o emitiu, que é o título da caixa no terminal.
  // No `claude -p` a ponte do terminal é nula e o `post` não sai: o aviso chega pelo `ui_toast`.
  on("ui.toast", async ($, e, next) => {
    void post($, "toast", fields({ text: e.text, timeoutMs: e.timeoutMs, plugin: originOf(next) }));
    return next(e);
  });

  on("ui.press", async ($, e, next) => {
    // A ponte do aparelho do clique: a do terminal, ou a da superfície `desktop` que o Hangar liga no
    // `claude -p`. Outra superfície (o app da Anthropic pelo Remote Control) segue sem janela.
    const ponte = e.surface === "terminal" ? bridge() : e.surface === "desktop" ? surfaceBridge() : null;
    if (e.surface === "terminal" || ponte) {
      const attempt = ponte ? await fromApp($, e.requestId, e.element, ponte) : null;
      appPress = attempt && ponte ? { until: (await $.clock.now()) + APP_PRESS_MS, plugin: e.plugin, attempt, surface: e.surface, ponte } : null;
    }
    try {
      return await next(e);
    } finally {
      if (e.surface === "terminal") void tell($, "pressed", { requestId: e.requestId, element: e.element });
    }
  });

  on("ui.copy", async ($, e, next) => {
    const clique = await appAttempt($, originOf(next));
    // Na superfície `desktop` a cópia já vai ao Hangar pelo `ui_copy`: desviá-la aqui a mandaria duas vezes.
    if (!clique || clique.surface !== "terminal" || !(await tell($, "copied", { attempt: clique.attempt, text: e.text }, clique.ponte))) return next(e);
    return { value: { isCopied: true } };
  });

  on("process.run", async ($, e, next) => {
    const url = openerUrl(e.argv);
    const clique = url ? await appAttempt($, originOf(next)) : null;
    if (!url || !clique || !(await tell($, "opened", { attempt: clique.attempt, url }, clique.ponte))) return next(e);
    return { value: { exitCode: 0, stdout: "", stderr: "", isStdoutTruncated: false, isStderrTruncated: false } };
  });
}
