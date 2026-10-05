import { describe, expect, it } from 'vitest';
import { formataErro } from './errosApi';
import * as m from './paraglide/messages';

describe('Git/arquivos recusado pelo Rust', () => {
  it('traduz os três códigos e leva o motivo para a frase', () => {
    expect(formataErro({ code: 'workspace_busy', params: { motivo: 'vagas cheias' }, msg: 'x' })).toBe(m.workspace_busy());
    for (const code of ['workspace_context', 'workspace_unavailable']) {
      const texto = formataErro({ code, params: { motivo: 'git não encontrado' }, msg: 'x' });
      expect(texto).toContain('git não encontrado');
      expect(texto).not.toBe('x');
    }
  });
});

describe('histórico recusado pelo Rust', () => {
  it('traduz os três códigos e mantém o código na frase', () => {
    for (const code of ['internal_info', 'history_io', 'history_panic']) {
      const texto = formataErro({ code, params: { motivo: 'frase em português' }, msg: 'x' });
      expect(texto).toContain(code);
      expect(texto).not.toBe('x');
    }
  });
});
