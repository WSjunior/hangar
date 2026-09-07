import { describe, it, expect } from 'vitest';
import { proposedPlan, planDisplayText } from './proposedPlan';

describe('plano proposto pelo Codex', () => {
  it('extrai o plano completo preservando o Markdown', () => {
    const text = 'Segue o plano.\n<proposed_plan>\n# Plano\n\n- **Primeiro**\n</proposed_plan>';
    expect(proposedPlan(text)).toBe('# Plano\n\n- **Primeiro**');
    expect(planDisplayText(text)).toBe('Segue o plano.\n\n# Plano\n\n- **Primeiro**\n');
  });
  it('não oferece aprovação durante a escrita', () => {
    expect(proposedPlan('<proposed_plan>\n# Incompleto')).toBeNull();
    expect(planDisplayText('<proposed_plan>\n# Incompleto')).toBe('\n# Incompleto');
  });
  it('preserva exemplos de tags dentro de blocos de código', () => {
    const example = '```xml\n<proposed_plan>\ntexto\n</proposed_plan>\n```';
    expect(proposedPlan(example)).toBeNull();
    expect(planDisplayText(example)).toBe(example);
  });
  it('reconhece a tag final sem abertura no trecho recebido', () => {
    expect(proposedPlan('# Plano\n\nEtapas.\n</proposed_plan>')).toBe('# Plano\n\nEtapas.');
  });
});
