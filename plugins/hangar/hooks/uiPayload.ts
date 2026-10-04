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
  const corpo = (a: unknown, p: readonly PaneEntry[]) =>
    `"above":${JSON.stringify(a)},"columns":${columns ?? "null"},"panes":${JSON.stringify(p)}`;
  const tudo = corpo(faixa, panes);
  if (tudo.length <= max) return tudo;
  const soFaixa = corpo(faixa, []);
  return soFaixa.length <= max ? soFaixa : corpo(null, []);
}
