import type { EngineInterface, On } from "claude-code";
import { type Bridge as Ponte, bridge } from "./bridge";

type DoApp = { answers?: Record<string, string> | null; deny?: string | null;
  publication_id?: string; generation?: number; session_id?: string };

/** Pergunta de múltipla escolha respondida pelo app SEM tecla no terminal.
 *
 *  O diálogo do terminal e o app correm juntos: quem responder primeiro vale.
 *  `next(e)` abre o diálogo de sempre; devolver antes dele resolver aborta o
 *  que roda embaixo, e o diálogo fecha sozinho. Sem ponte (ou com ela fora do
 *  ar) sobra só o terminal, como antes. */
export function registerAsk(on: On) {
  on("tool.call", { tool: "AskUserQuestion" }, async ($, e, next) => {
    const ponte = bridge();
    if (!ponte) return next(e);
    const id = `ask:${e.tool_use_id ?? ""}`;
    const questions = e.questions;

    let acabou = false;
    const terminal = next(e).then((r) => ({ de: "terminal" as const, r }));
    // Quando o app vence, o diálogo abortado embaixo pode REJEITAR depois do retorno: sem isto
    // sobraria uma rejeição sem dono no worker. A corrida continua vendo a rejeição normalmente.
    terminal.catch(() => {});
    const app = doApp($, ponte, id, questions, () => acabou).then((r) => ({ de: "app" as const, r }));
    let vencedor: "terminal" | "app" | "erro" = "erro";
    let receipt: DoApp = {};
    try {
      const v = await Promise.race([terminal, app]);
      vencedor = v.de;
      if (v.de === "terminal") return v.r;
      receipt = v.r;
      if (v.r.deny) return { deny: v.r.deny };
      return { result: { questions, answers: v.r.answers } };
    } finally {
      acabou = true;
      await fim($, ponte, id, vencedor, receipt);
    }
  });
}

// Backend reiniciando derruba o long-poll em voo. Insistir é de graça: o relógio
// do orçamento fica parado enquanto o `next(e)` do diálogo está pendente.
const RETRY_MS = 2000;

/** Segura um long-poll até o app responder. Token recusado vira espera sem
 *  fim: quem decide, nesse caso, é o diálogo do terminal. */
async function doApp(
  $: EngineInterface,
  ponte: Ponte,
  id: string,
  questions: unknown,
  acabou: () => boolean,
): Promise<DoApp> {
  while (!acabou()) {
    let status = 0;
    let texto = "";
    try {
      const r = await $.http.fetch(`${ponte.url}/ask`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ sessao: ponte.sessao, token: ponte.token, id, questions }),
      });
      status = r.status;
      texto = r.text;
    } catch {
      // cai no retry abaixo
    }
    if (status === 403) break;
    if (status !== 200) {
      await $.clock.sleep(RETRY_MS);
      continue;
    }
    let corpo: DoApp;
    try {
      corpo = JSON.parse(texto) as DoApp;
    } catch {
      // Corpo ilegível não pode derrubar a corrida: o diálogo do terminal já está aberto.
      await $.clock.sleep(RETRY_MS);
      continue;
    }
    if (corpo.answers || corpo.deny) return corpo;
  }
  return new Promise<DoApp>(() => {});
}

async function fim($: EngineInterface, ponte: Ponte, id: string, vencedor: string, receipt: DoApp) {
  try {
    await $.http.fetch(`${ponte.url}/ask-fim`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessao: ponte.sessao, token: ponte.token, id, vencedor,
        publication_id: receipt.publication_id, generation: receipt.generation, session_id: receipt.session_id }),
    });
  } catch {
    // A ponte fora do ar não pode derrubar a resposta que já existe.
  }
}
