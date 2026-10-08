export type QuestionPart = { kind: 'text' | 'code' | 'link'; text: string };

// Só http(s) sem espaço; pontuação final fica de fora do link.
const URL_RE = /https?:\/\/[^\s]+/g;
const TRAILING = /[:.,;!?)\]}'"]+$/;

/** Parte a pergunta do cartão em texto, `code` (entre crases) e links. */
export function questionParts(question: string): QuestionPart[] {
  const out: QuestionPart[] = [];
  const push = (kind: QuestionPart['kind'], text: string) => { if (text) out.push({ kind, text }); };
  question.split('`').forEach((chunk, i) => {
    if (i % 2 === 1) return push('code', chunk);
    let last = 0;
    for (const hit of chunk.matchAll(URL_RE)) {
      const url = hit[0].replace(TRAILING, '');
      if (url === 'http://' || url === 'https://') continue;
      push('text', chunk.slice(last, hit.index));
      push('link', url);
      last = hit.index + url.length;
    }
    push('text', chunk.slice(last));
  });
  return out;
}
