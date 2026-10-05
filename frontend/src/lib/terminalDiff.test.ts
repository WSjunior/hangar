import { describe, expect, it } from 'vitest';
import { betweenHunks, piecesOf } from './terminalDiff';

describe('piecesOf', () => {
  it('sem realce nem marca devolve a linha inteira', () => {
    expect(piecesOf('abc', null, [])).toEqual([{ content: 'abc', color: undefined, mark: false }]);
  });

  it('caminho rápido: sem realce nem marca é uma peça só, e linha vazia não tem peça', () => {
    expect(piecesOf('abc', [], [])).toEqual([{ content: 'abc', color: undefined, mark: false }]);
    expect(piecesOf('', null, [])).toEqual([]);
  });

  it('marca cortando o token de sintaxe no meio', () => {
    const tokens = [{ content: 'const ', color: '#1' }, { content: 'T = 8000', color: '#2' }, { content: ';' }];
    expect(piecesOf('const T = 8000;', tokens, [[10, 14]])).toEqual([
      { content: 'const ', color: '#1', mark: false },
      { content: 'T = ', color: '#2', mark: false },
      { content: '8000', color: '#2', mark: true },
      { content: ';', color: undefined, mark: false },
    ]);
  });

  it('tokens que não fecham com o texto são descartados', () => {
    expect(piecesOf('abc', [{ content: 'ab', color: '#1' }], [[0, 1]])).toEqual([
      { content: 'a', color: undefined, mark: true },
      { content: 'bc', color: undefined, mark: false },
    ]);
  });

  it('marca além do fim da linha é aparada', () => {
    expect(piecesOf('ab', null, [[1, 9]])).toEqual([
      { content: 'a', color: undefined, mark: false },
      { content: 'b', color: undefined, mark: true },
    ]);
  });
});

describe('betweenHunks', () => {
  it('mesmo arquivo ou sem caminhos dá o intervalo; arquivo diferente dá o nome', () => {
    expect(betweenHunks(undefined, 1)).toEqual({ kind: 'gap' });
    expect(betweenHunks(['/a/x.py', '/a/x.py'], 1)).toEqual({ kind: 'gap' });
    expect(betweenHunks(['/a/x.py', '/b/y.py'], 1)).toEqual({ kind: 'file', name: 'y.py', path: '/b/y.py' });
  });
});
