import type { EngineInterface, On } from "claude-code";
import { bridge } from "./bridge";

// Mesmo motivo do state.ts: `$` não atravessa import, então o envio é local.
async function enviar($: EngineInterface, tokens: number, seconds: number) {
  const p = bridge();
  if (!p) return;
  try {
    await $.http.fetch(`${p.url}/rate`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ sessao: p.sessao, token: p.token, tokens, seconds }),
    });
  } catch {
    // Backend fora do ar: perde-se a medida, nunca um pedaço da resposta.
  }
}

/** Velocidade de geração: do primeiro pedaço da resposta ao último, com o `output_tokens` real
 * que chega no fim. A espera antes do primeiro pedaço fica de fora de propósito. */
export function registerRate(on: On) {
  on("turn.step", async function* ($, e, next) {
    // Subagente roda em paralelo ao principal: misturar os relógios falsearia a medida.
    if (e.agentId) return yield* next(e);
    let inicio = 0;
    let final: [number, number] | null = null;
    for await (const c of next(e)) {
      const agora = Date.now();
      if (!inicio) inicio = agora;
      if (c.kind === "stop" && c.usage?.output_tokens) final = [c.usage.output_tokens, (agora - inicio) / 1000];
      yield c;
    }
    if (final) await enviar($, final[0], final[1]);
  });
}
