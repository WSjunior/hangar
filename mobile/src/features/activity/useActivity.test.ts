import { describe, it, expect } from 'vitest';
import { createActivityFolder } from '@hangar/core';
import type { ChatEvent } from '@hangar/core';
import { feedFolder } from './useActivity';

// teste do Step 1 da Task 9: 3 tool_use → agents.length === 3; reset([]) zera
function agentEvent(toolUseId: string, seq: number): ChatEvent {
  return {
    kind: 'tool_use',
    id: `e${seq}`,
    tool_name: 'Agent',
    tool_use_id: toolUseId,
    tool_input: { description: `Agent ${seq}`, prompt: `prompt-${seq}` },
  } as ChatEvent;
}

describe('useActivity — createActivityFolder incremental', () => {
  it('3 tool_use Agent → agents.length === 3; reset([]) zera', () => {
    const folder = createActivityFolder();
    folder.push(agentEvent('tu1', 1));
    folder.push(agentEvent('tu2', 2));
    folder.push(agentEvent('tu3', 3));
    expect(folder.snapshot().agents.length).toBe(3);
    folder.reset([]);
    const after = folder.snapshot();
    expect(after.agents.length).toBe(0);
    expect(after.total).toBe(0);
    expect(after.tasks.length).toBe(0);
  });

  it('histórico antigo entrando na frente refaz a pasta, sem repetir a cauda', () => {
    const folder = createActivityFolder();
    let cursor = feedFolder(folder, { count: 0, firstId: undefined }, [agentEvent('n1', 3)]);
    cursor = feedFolder(folder, cursor, [agentEvent('o1', 1), agentEvent('o2', 2), agentEvent('n1', 3)]);
    expect(folder.snapshot().agents.map((a) => a.description).sort()).toEqual(['Agent 1', 'Agent 2', 'Agent 3']);
    cursor = feedFolder(folder, cursor, [agentEvent('o1', 1), agentEvent('o2', 2), agentEvent('n1', 3), agentEvent('n2', 4)]);
    expect(folder.snapshot().agents.length).toBe(4);
  });

  it('reset com 3 eventos e depois reset vazio zera incremental', () => {
    const folder = createActivityFolder();
    const events: ChatEvent[] = [agentEvent('a1', 1), agentEvent('a2', 2), agentEvent('a3', 3)];
    folder.reset(events);
    expect(folder.snapshot().agents.length).toBe(3);
    folder.reset([]);
    expect(folder.snapshot().agents.length).toBe(0);
  });
});
