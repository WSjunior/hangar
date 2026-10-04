import type { EngineInterface, On } from "claude-code";
import { type Bridge as Ponte, bridge } from "./bridge";

type DoApp = { permitir?: boolean | null; soltar?: boolean;
  publication_id?: string; generation?: number; session_id?: string };

// `tool.check` roda ANTES do diálogo de permissão: enquanto este hook segura o
// `ask`, o terminal não mostra nada. Por isso quem decide se segura é o backend
// (alguém no app E ninguém no terminal), reperguntado a cada janela — prender um
// terminal no meio da espera devolve o diálogo a ele em poucos segundos.
const JANELA_MS = 5000;
const RESUMO_MAX = 400;
// Ferramentas cujo `ask` É o próprio diálogo (pergunta, aprovação de plano): segurar aqui
// esconderia a pergunta do terminal e do hook que corre por ela (ask.ts).
const SAO_DIALOGO = new Set(["AskUserQuestion", "ExitPlanMode", "EnterPlanMode"]);

/** Pedido de permissão respondido pelo app sem tecla no terminal. */
export function registerPerm(on: On) {
  on("tool.check", async ($, e, next) => {
    const base = await next(e);
    if (base.decision !== "ask" || !e.tool_use_id || SAO_DIALOGO.has(e.tool)) return base;
    const ponte = bridge();
    if (!ponte) return base;
    const id = `perm:${e.tool_use_id}`;

    const r = await doApp($, ponte, id, e.tool, resumo(e.input));
    await fim($, ponte, id, r ? "app" : "terminal", r);
    if (r === null) return base;
    if (r.permitir) return { decision: "allow" as const, reason: "Aprovado no app Hangar." };
    return { decision: "deny" as const, reason: "Negado pelo usuário no app Hangar." };
  });
}

function resumo(input: unknown): string {
  const i = (input ?? {}) as Record<string, unknown>;
  const s = String(i.command ?? i.file_path ?? i.url ?? JSON.stringify(i));
  return s.length > RESUMO_MAX ? `${s.slice(0, RESUMO_MAX)}…` : s;
}

/** null = o diálogo é do terminal (backend mandou soltar, ou a ponte falhou). */
async function doApp(
  $: EngineInterface,
  ponte: Ponte,
  id: string,
  tool: string,
  texto: string,
): Promise<DoApp | null> {
  for (;;) {
    let corpo: DoApp;
    try {
      const r = await $.http.fetch(`${ponte.url}/ask`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          sessao: ponte.sessao, token: ponte.token, id, tool, resumo: texto, janela_ms: JANELA_MS,
        }),
      });
      if (r.status !== 200) return null;
      corpo = JSON.parse(r.text) as DoApp;
    } catch {
      return null;
    }
    if (corpo.soltar) return null;
    if (typeof corpo.permitir === "boolean") return corpo;
  }
}

async function fim($: EngineInterface, ponte: Ponte, id: string, vencedor: string, receipt: DoApp | null) {
  try {
    await $.http.fetch(`${ponte.url}/ask-fim`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessao: ponte.sessao, token: ponte.token, id, vencedor,
        publication_id: receipt?.publication_id, generation: receipt?.generation, session_id: receipt?.session_id }),
    });
  } catch {
    // A ponte fora do ar não pode derrubar o veredito que já existe.
  }
}
