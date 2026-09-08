import { describe, it, expect } from 'vitest';
import { lerSubagenteCodex } from './subagenteCodex';

// Linha real do rollout (`~/.codex/sessions/.../rollout-*.jsonl`), encurtada.
const CONCLUIDO = '<subagent_notification>\n'
  + '{"agent_path":"01a07da0-4079-7223-a308-8d21a717ad19","status":'
  + '{"completed":"Encontrei 3 lacunas.\\n\\n1. **Troca `/new`** não reata."}}\n'
  + '</subagent_notification>';

const ERRO = '<subagent_notification>\n'
  + '{"agent_path":"01a07c11","status":{"errored":"You\'ve hit your usage limit."}}\n'
  + '</subagent_notification>';

describe('lerSubagenteCodex', () => {
  it('lê o relatório com as quebras de linha desescapadas', () => {
    expect(lerSubagenteCodex(CONCLUIDO)).toEqual({
      status: 'completed',
      agentPath: '01a07da0-4079-7223-a308-8d21a717ad19',
      texto: 'Encontrei 3 lacunas.\n\n1. **Troca `/new`** não reata.',
    });
  });

  it('lê o erro', () => {
    expect(lerSubagenteCodex(ERRO)?.status).toBe('errored');
  });

  it('devolve null pra mensagem comum e pra JSON quebrado', () => {
    expect(lerSubagenteCodex('olha o <subagent_notification> que apareceu')).toBeNull();
    expect(lerSubagenteCodex('<subagent_notification>\n{nao é json\n</subagent_notification>')).toBeNull();
    expect(lerSubagenteCodex('<subagent_notification>\n{"agent_path":"x"}\n</subagent_notification>')).toBeNull();
  });
});
