import { expect, it } from 'vitest';
import { replaceSlashToken, sideQuestionOf, slashMatches, slashTokenAt } from './slashCommands';
import type { CommandInfo } from './types';

const cmd = (name: string): CommandInfo => ({ name, display: `/${name}`, source: 'builtin' });

it('prefixo antes de substring, com teto', () => {
  const all = [cmd('recompact'), cmd('compact'), cmd('clear'), cmd('help')];
  expect(slashMatches(all, 'comp').map((c) => c.name)).toEqual(['compact', 'recompact']);
  expect(slashMatches(all, '', 2)).toHaveLength(2);
  expect(slashMatches(all, null)).toEqual([]);
});

it('acha o /nome sob o cursor em qualquer ponto do texto', () => {
  expect(slashTokenAt('/co', 3)).toEqual({ start: 0, end: 3, query: 'co', whole: true });
  expect(slashTokenAt('  /co', 5)).toEqual({ start: 2, end: 5, query: 'co', whole: true });
  expect(slashTokenAt('revise com /sim', 15)).toEqual({ start: 11, end: 15, query: 'sim', whole: false });
  expect(slashTokenAt('a\n/', 3)).toEqual({ start: 2, end: 3, query: '', whole: false });
  expect(slashTokenAt('pmedico:help', 12)).toBeNull();
  expect(slashTokenAt('/pmedico:help', 13)?.query).toBe('pmedico:help');
  // Cursor no meio da palavra: a busca vai até o cursor, a troca pega a palavra inteira.
  expect(slashTokenAt('/compact agora', 3)).toEqual({ start: 0, end: 8, query: 'co', whole: false });
});

it('caminho, URL, argumento e cursor fora da palavra não abrem a lista', () => {
  expect(slashTokenAt('abre /home/user/x', 17)).toBeNull();
  expect(slashTokenAt('veja https://a/b', 16)).toBeNull();
  expect(slashTokenAt('src/app', 7)).toBeNull();
  expect(slashTokenAt('/compact agora', 14)).toBeNull();
  expect(slashTokenAt('/compact ', 9)).toBeNull();
  expect(slashTokenAt('a /co', 2)).toBeNull();
  expect(slashTokenAt('', 0)).toBeNull();
});

it('troca só a palavra do token', () => {
  const text = 'revise com /sim';
  expect(replaceSlashToken(text, slashTokenAt(text, 15)!, 'simplify')).toEqual({ text: 'revise com /simplify ', cursor: 21 });
  const middle = 'use /co e depois';
  expect(replaceSlashToken(middle, slashTokenAt(middle, 7)!, 'compact')).toEqual({ text: 'use /compact e depois', cursor: 13 });
  const line = 'use /co\nfim';
  expect(replaceSlashToken(line, slashTokenAt(line, 7)!, 'compact')).toEqual({ text: 'use /compact \nfim', cursor: 13 });
});

it('reconhece /btw com e sem pergunta', () => {
  expect(sideQuestionOf('/btw o que falta?')).toBe('o que falta?');
  expect(sideQuestionOf('/BTW')).toBe('');
  expect(sideQuestionOf('/btwx')).toBeNull();
  expect(sideQuestionOf('oi')).toBeNull();
});
