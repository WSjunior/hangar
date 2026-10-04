/** Um painel que um mod abriu e o terminal desenhou, como vai ao backend. */
export type PaneEntry = {
  id: string;
  title: string;
  placement: "dock" | "inline";
  columns: number;
  tree: unknown;
};

const isEngine = (tree: unknown) => !tree || (tree as { type?: string }).type === "engine";

/** Miolo do corpo do POST /ui (sem sessão e token). Acima de `max` caracteres saem primeiro os
 *  painéis, depois a faixa: a faixa é o que todo mod tem. */
export function bandBody(above: unknown, columns: number | null, panes: readonly PaneEntry[], max: number): string {
  const faixa = isEngine(above) ? null : above;
  for (const [a, p] of [[faixa, panes], [faixa, []], [null, []]] as const) {
    const json = `"above":${JSON.stringify(a)},"columns":${columns ?? "null"},"panes":${JSON.stringify(p)}`;
    if (json.length <= max) return json;
  }
  return `"above":null,"columns":null,"panes":[]`;
}
