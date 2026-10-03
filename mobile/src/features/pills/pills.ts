import type { StatusFields } from '@hangar/core';

// Escolha otimista da linha do composer: vale até a statusline confirmar (modelo) ou para sempre
// (nível, que não tem leitura de volta confiável).
export type Chosen = { model?: string | null; effort?: string | null };

// rótulo que a pílula mostra: escolhido otimista prevalece sobre o que veio da statusline
export function pillLabels(
  f: StatusFields | null,
  chosen: { model?: string | null; effort?: string | null },
): { model: string | null; effort: string | null } {
  return {
    model: chosen.model ?? f?.model ?? null,
    effort: chosen.effort ?? f?.effort ?? null,
  };
}

// quando a statusline confirma o que foi escolhido (substring), solta o otimista
// porte de Composer.svelte:571-577 — só o modelo tem read-back confiável, esforço é write-only
export function reconcileChosen(
  f: StatusFields | null,
  chosen: { model?: string | null; effort?: string | null },
): { model?: string | null; effort?: string | null } {
  if (!f?.model || !chosen.model) return chosen;
  // Pela primeira palavra: a lista diz "Opus 5.5·1M" e a statusline, "Opus5.5·1M".
  const word = chosen.model.toLowerCase().split(/[\s·[(]/)[0];
  if (word && f.model.toLowerCase().includes(word)) {
    return { ...chosen, model: null };
  }
  return chosen;
}

// haiku não usa esforço (picker responde "Effort not supported")
export function semEsforco(model: string | null | undefined): boolean {
  return !!model && model.toLowerCase().includes('haiku');
}

const CLAUDE_EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max', 'ultracode'];

// Níveis que o picker do Claude oferece por modelo (medição em backend/app/model_picker.py):
// Sonnet tem só quatro, Haiku nenhum. Modelo desconhecido recebe a lista inteira, como antes.
// ponytail: casa pelo nome; trocar pela lista lida do picker quando o backend expuser uma.
export function claudeEfforts(model: string | null | undefined): string[] {
  if (semEsforco(model)) return [];
  if (model?.toLowerCase().includes('sonnet')) return ['low', 'medium', 'high', 'max'];
  return CLAUDE_EFFORTS;
}
