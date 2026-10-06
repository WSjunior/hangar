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

describe('recusas novas dos mods', () => {
  it.each([
    'erro_mod_sem_digitacao',
    'erro_mod_desenho_vencido',
    'erro_mod_dialogo_aberto',
    'erro_mod_rascunho_no_prompt',
    'erro_mod_painel_nao_alcancavel',
    'erro_mod_fechar_recusado',
    'erro_mod_guarda_indisponivel',
    'session_transfer_busy',
    'erro_mod_painel_inexistente',
  ])('%s vira frase do app, não o texto do servidor', (code) => {
    const texto = formataErro({ code, params: {}, msg: 'texto-do-servidor' });
    expect(texto).not.toBe('texto-do-servidor');
    expect(texto).not.toBe(code);
  });
});
