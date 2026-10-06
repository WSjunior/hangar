// Reserva por teclado do clique do app (T5): o backend arma um alvo, o hook de `ui.focus` põe a `key`
// dele no evento e conta onde o anel pousou. A reescrita só leva o anel a outro elemento do mesmo mod,
// e só quando o movimento já ia para um elemento (as paradas do motor vêm sem `plugin`), medido em (r).

/** O que a rota `focus-target` devolve com um alvo armado. */
export type FocusTarget = { attempt: string; rewrite: string | null };

/** O `element` que segue no evento: a `key` armada, quando cabe; senão, o do próprio evento. */
export function focusElement(e: { plugin?: string; element?: string }, target: FocusTarget | null): string | undefined {
  if (!target?.rewrite || !e.plugin || !e.element) return e.element;
  return target.rewrite;
}

/** Por quanto tempo, depois de um alvo armado, o envio do composer fica segurado: uma letra digitada
 *  entre o `Tab` e o `Enter` cai no rascunho e o `Enter` seguinte o enviaria ((aa), achado 13). */
export const HOLD_MS = 3000;

export function holding(now: number, until: number | null): boolean {
  return until !== null && now < until;
}
