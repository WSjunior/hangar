import type { AnswerItem, AskQuestionItem } from '@hangar/core';

export type PickState =
  | { kind: 'option'; indices: number[] }
  | { kind: 'text'; value: string }
  | { kind: 'chat' };

// Porte literal de AskQuestionStepper.svelte:89-98 — é o payload que POST /answer valida.
export function buildAnswers(questions: AskQuestionItem[], picks: PickState[]): AnswerItem[] {
  return questions.map((q, qi) => {
    const p = picks[qi] ?? { kind: 'option', indices: [] };
    const identity = q.id !== undefined ? { question_id: q.id } : {};
    if (p.kind === 'text') return { ...identity, kind: 'text', value: p.value, type_index: q.options.length, labels: [p.value] };
    if (p.kind === 'chat') return { ...identity, kind: 'chat', chat_index: q.options.length + 1 };
    return { ...identity, kind: 'option', indices: p.indices, multi: q.multiSelect, labels: p.indices.map((i) => q.options[i].label) };
  });
}
