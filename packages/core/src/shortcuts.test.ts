import { describe, it, expect } from 'vitest';
import {
  defaultShortcuts,
  mergeProjectShortcuts,
  resolveShortcuts,
  serializeShortcuts,
  sendsDirect,
  runsInHangar,
  hangarHome,
  answersInApp,
  hangarKeyOf,
  type Shortcut,
  type ShortcutSendText,
  type ShortcutShell,
} from './shortcuts';

describe('resolveShortcuts', () => {
  it('config vazia resolve no conjunto nativo, na ordem de hoje', () => {
    for (const raw of ['', '   ', null, undefined]) {
      expect(resolveShortcuts(raw)).toEqual(defaultShortcuts());
    }
    expect(defaultShortcuts().map((s) => s.id)).toEqual([
      'terminal', 'modo', 'navegador', 'anexos', 'rodar', 'externo',
    ]);
  });

  it('JSON quebrado ou shape estranho NUNCA lança: cai no conjunto nativo', () => {
    expect(resolveShortcuts('{quebrado')).toEqual(defaultShortcuts());
    expect(resolveShortcuts('{"id":"x"}')).toEqual(defaultShortcuts());
    expect(resolveShortcuts('"texto"')).toEqual(defaultShortcuts());
  });

  it('item individual inválido é descartado sem derrubar a lista', () => {
    const list: Shortcut[] = [
      { id: 'terminal', type: 'internal', action: 'terminal' },
      { id: 'a1', type: 'send_text', label: 'Relatório', text: '/relatorio-pm' },
    ];
    const raw = JSON.stringify([...list, { id: 'x', type: 'foguete' }, { type: 'shell' }]);
    expect(resolveShortcuts(raw)).toEqual(list);
  });

  it('opcional com tipo errado descarta o item; id repetido fica só o primeiro', () => {
    const ok: Shortcut = { id: 'a1', type: 'shell', label: 'Editor', command: 'code .' };
    const raw = JSON.stringify([
      ok,
      { id: 'a2', type: 'shell', label: 'X', command: 'x', icon: 42 },
      { id: 'a3', type: 'send_text', label: 'Y', text: '/y', send_direct: 'nao' },
      { ...ok, label: 'Duplicado' },
    ]);
    expect(resolveShortcuts(raw)).toEqual([ok]);
  });

  it('roundtrip serialize → resolve preserva a lista', () => {
    const list: Shortcut[] = [
      { id: 'a2', type: 'shell', label: 'Editor', command: 'code .', confirm: true },
      { id: 'rodar', type: 'internal', action: 'rodar' },
    ];
    expect(resolveShortcuts(serializeShortcuts(list))).toEqual(list);
  });
});

describe('pasta do atalho shell', () => {
  it('sobrevive ao roundtrip; pasta vazia ou de outro tipo descarta o item', () => {
    const ok: Shortcut = { id: 'a1', type: 'shell', label: 'Debug', command: 'npm run debug', pasta: 'frontend' };
    const raw = JSON.stringify([
      ok,
      { id: 'a2', type: 'shell', label: 'X', command: 'x', pasta: '  ' },
      { id: 'a3', type: 'shell', label: 'Y', command: 'y', pasta: 3 },
    ]);
    expect(resolveShortcuts(raw)).toEqual([ok]);
    expect(resolveShortcuts(serializeShortcuts([ok]))).toEqual([ok]);
  });
});

describe('mergeProjectShortcuts', () => {
  const global: Shortcut[] = [{ id: 'terminal', type: 'internal', action: 'terminal' }];
  it('globais primeiro, depois os do projeto; mesmo id nas duas listas não colide na chave', () => {
    const own: Shortcut = { id: 'terminal', type: 'shell', label: 'Debug', command: 'd', pasta: '/tmp' };
    expect(mergeProjectShortcuts(global, [own])).toEqual([
      { shortcut: global[0], scope: 'global', key: 'global:terminal' },
      { shortcut: own, scope: 'project', key: 'project:terminal' },
    ]);
  });
  it('do projeto: interno, inválido e id repetido caem fora; sem lista = só globais', () => {
    const project = [
      { id: 'modo', type: 'internal', action: 'modo' },
      { id: 'p1', type: 'send_text', label: 'X', text: '/x' },
      { id: 'p1', type: 'send_text', label: 'Y', text: '/y' },
      { id: 'p2', type: 'shell', label: 'Z' },
    ];
    expect(mergeProjectShortcuts(global, project).map((e) => e.key))
      .toEqual(['global:terminal', 'project:p1']);
    expect(mergeProjectShortcuts(global, null)).toHaveLength(1);
  });
});

describe('sendsDirect', () => {
  const base: ShortcutSendText = { id: 'a', type: 'send_text', label: 'X', text: '/x' };
  it('flag ausente = envia direto; só false pré-preenche', () => {
    expect(sendsDirect(base)).toBe(true);
    expect(sendsDirect({ ...base, send_direct: true })).toBe(true);
    expect(sendsDirect({ ...base, send_direct: false })).toBe(false);
  });
});

describe('campos do shell No Hangar', () => {
  it('mantém os campos do shell e descarta valor desconhecido', () => {
    const raw = JSON.stringify([
      { id: 'a', type: 'shell', label: 'RDP', command: 'delphi-vm', runs_in: 'hangar', hangar_home: false, answer_in_app: false },
      { id: 'b', type: 'shell', label: 'X', command: 'x', runs_in: 'nuvem' },
      { id: 'c', type: 'shell', label: 'Y', command: 'y', answer_in_app: 'sim' },
    ]);
    const list = resolveShortcuts(raw) as ShortcutShell[];
    expect(list.map((s) => s.id)).toEqual(['a']);
    expect(runsInHangar(list[0])).toBe(true);
    expect(hangarHome(list[0])).toBe(false);
    expect(answersInApp(list[0])).toBe(false);
    const plain: ShortcutShell = { id: 'd', type: 'shell', label: 'Z', command: 'z' };
    expect([runsInHangar(plain), hangarHome(plain), answersInApp(plain)]).toEqual([false, true, true]);
    expect(hangarKeyOf('global', 'a')).toBe('global:a');
    expect(hangarKeyOf('project', 'a', '/repo/x')).toBe('project:/repo/x:a');
  });
});
