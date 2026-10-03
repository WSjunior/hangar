import { describe, expect, it } from 'vitest';
import { buildCreateSessionBody, pickFolderRoot, uniqueSessionName } from './api';

describe('uniqueSessionName', () => {
  it('sanitiza como o backend e desempata com -2', () => {
    expect(uniqueSessionName('Área de trabalho', new Set())).toBe('Area-de-trabalho');
    expect(uniqueSessionName('hangar', new Set(['hangar', 'hangar-2']))).toBe('hangar-3');
    expect(uniqueSessionName('***', new Set())).toBe('sessao');
  });
});

describe('buildCreateSessionBody branch', () => {
  it('repassa a branch só quando preenchida', () => {
    expect(buildCreateSessionBody({ name: 'a', branch: 'dev' }).branch).toBe('dev');
    expect('branch' in buildCreateSessionBody({ name: 'a', branch: '' })).toBe(false);
    expect('branch' in buildCreateSessionBody({ name: 'a', branch: null })).toBe(false);
  });
  it('leva branch nova e base só quando new_branch', () => {
    expect(buildCreateSessionBody({ name: 'a', branch: 'nova', new_branch: true, base: 'main' }))
      .toEqual({ name: 'a', branch: 'nova', new_branch: true, base: 'main' });
    expect(buildCreateSessionBody({ name: 'a', branch: 'x', new_branch: false, base: 'main' }))
      .toEqual({ name: 'a', branch: 'x' });
  });
});

describe('pickFolderRoot', () => {
  const r = (...p: string[]) => p.map(path => ({ path }));
  it('respeita a fronteira do diretório', () => {
    expect(pickFolderRoot(r('/home/a'), '/home/ab')).toBeNull();
    expect(pickFolderRoot(r('/home/a'), '/home/a/x')).toBe('/home/a');
    expect(pickFolderRoot(r('/home/a'), '/home/a')).toBe('/home/a');
  });
  it('entende caminho do Windows e devolve a raiz original', () => {
    expect(pickFolderRoot(r('C:\\proj'), 'c:\\proj\\x')).toBe('C:\\proj');
    expect(pickFolderRoot(r('C:\\proj'), 'C:\\project')).toBeNull();
    expect(pickFolderRoot(r('C:\\proj\\'), 'C:/proj/x')).toBe('C:\\proj\\');
  });
  it('a raiz mais longa vence', () => {
    expect(pickFolderRoot(r('/home', '/home/a'), '/home/a/x')).toBe('/home/a');
  });
});
