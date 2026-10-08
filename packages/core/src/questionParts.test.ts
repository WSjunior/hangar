import { describe, expect, test } from 'vitest';
import { questionParts } from './questionParts';

describe('questionParts', () => {
  test('texto sem URL fica igual', () => {
    expect(questionParts('Aprovar o plano?')).toEqual([{ kind: 'text', text: 'Aprovar o plano?' }]);
  });
  test('URL vira link e a pontuação final fica fora', () => {
    expect(questionParts('srv pede para abrir https://a.com/x?y=1: confirme.')).toEqual([
      { kind: 'text', text: 'srv pede para abrir ' },
      { kind: 'link', text: 'https://a.com/x?y=1' },
      { kind: 'text', text: ': confirme.' },
    ]);
    expect(questionParts('(veja http://a.com/b).')).toEqual([
      { kind: 'text', text: '(veja ' },
      { kind: 'link', text: 'http://a.com/b' },
      { kind: 'text', text: ').' },
    ]);
  });
  test('crases continuam code e não viram link', () => {
    expect(questionParts('rode `curl https://a.com` e abra https://b.com')).toEqual([
      { kind: 'text', text: 'rode ' },
      { kind: 'code', text: 'curl https://a.com' },
      { kind: 'text', text: ' e abra ' },
      { kind: 'link', text: 'https://b.com' },
    ]);
  });
  test('esquemas que não são http(s) não viram link', () => {
    expect(questionParts('javascript:alert(1) ftp://a.com')).toEqual([{ kind: 'text', text: 'javascript:alert(1) ftp://a.com' }]);
  });
});
