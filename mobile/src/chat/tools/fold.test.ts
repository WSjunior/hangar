import { describe, expect, it } from 'vitest';
import { agruparConversa, toolGroupTitulo, type ChatEvent } from '@hangar/core';
import * as m from '../../paraglide/messages';
import { callTarget, failedLabel, foldConversation, foldTitle } from './fold';

const ev = (id: string, kind: ChatEvent['kind'], extra: Partial<ChatEvent> = {}): ChatEvent => ({ id, kind, ...extra });
const bash = (id: string) => ev(id, 'tool_use', { tool_name: 'Bash', tool_input: { command: 'ls' } });

describe('foldConversation', () => {
  it('junta pensamento, chamada solta e grupo seguidos num trecho só, entre mensagens', () => {
    const eventos = [
      ev('u1', 'user_msg', { text: 'oi' }),
      ev('t1', 'thinking', { text: 'pensando.' }),
      bash('b1'),
      bash('b2'),
      bash('b3'),
      ev('a1', 'assistant_msg', { text: 'feito' }),
      bash('b4'),
    ];
    const rows = foldConversation(agruparConversa(eventos, { entraNoPensamento: () => false }));
    expect(rows.map((r) => r.type)).toEqual(['event', 'fold', 'event', 'fold']);
    const fold = rows[1];
    expect(fold.type === 'fold' && fold.parts.map((p) => p.id)).toEqual(['t1', 'b1', 'b2', 'b3']);
    expect(fold.id).toBe('f-p-t1');
  });

  it('id do trecho não muda quando ele cresce na cauda', () => {
    const base = [ev('t1', 'thinking', { text: 'x' }), bash('b1')];
    const antes = foldConversation(agruparConversa(base, { entraNoPensamento: () => false }));
    const depois = foldConversation(agruparConversa([...base, bash('b2'), bash('b3')], { entraNoPensamento: () => false }));
    expect(depois[0].id).toBe(antes[0].id);
  });

  it('página publicada sai do trecho e corta a junção; sem página, segue linha comum', () => {
    const page = { id: 'pg', title: 'T', height: null, heights: {}, ownTheme: false };
    const render = ev('h1', 'tool_use', { tool_name: 'mcp__hangar__html_render', tool_use_id: 'h1' });
    const eventos = [bash('b1'), render, bash('b2')];
    const items = agruparConversa(eventos, { entraNoPensamento: () => false });
    const rows = foldConversation(items, true, (e) => (e.id === 'h1' ? page : null));
    expect(rows.map((r) => r.type)).toEqual(['fold', 'page', 'fold']);
    expect(rows[1].type === 'page' && rows[1].page).toBe(page);
    expect(foldConversation(items, true).map((r) => r.type)).toEqual(['fold']);
  });
});

describe('foldTitle', () => {
  it('raciocínio primeiro, depois a contagem em minúscula', () => {
    const parts = [ev('t1', 'thinking'), ev('t2', 'thinking'), bash('b1'), bash('b2')];
    const counts = toolGroupTitulo([{ tool_name: 'Bash' }, { tool_name: 'Bash' }]);
    expect(foldTitle(parts)).toBe(`${m.native_tree_thoughts({ n: 2 })} · ${counts.charAt(0).toLowerCase()}${counts.slice(1)}`);
  });

  it('o description do Bash não vira título e o ToolSearch não conta', () => {
    const parts = [
      ev('s', 'tool_use', { tool_name: 'ToolSearch' }),
      ev('b', 'tool_use', { tool_name: 'Bash', tool_input: { command: 'ls', description: 'Lista' } }),
    ];
    expect(foldTitle(parts)).toBe(toolGroupTitulo([{ tool_name: 'Bash' }]));
  });

  it('só raciocínio', () => {
    expect(foldTitle([ev('t1', 'thinking')])).toBe(m.native_thinking());
  });
});

describe('failedLabel e callTarget', () => {
  it('falha conta no singular e plural', () => {
    expect(failedLabel(0)).toBe('');
    expect(failedLabel(1)).toBe(m.native_tools_failed_1());
    expect(failedLabel(3)).toBe(m.native_tools_failed({ n: 3 }));
  });

  it('ferramenta de arquivo mostra só o nome; comando fica inteiro', () => {
    expect(callTarget('Read', '/repo/src/a.ts (offset=10)')).toBe('a.ts (offset=10)');
    expect(callTarget('Bash', 'cat /etc/hosts')).toBe('cat /etc/hosts');
  });
});
