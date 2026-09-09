import { describe, it, expect } from 'vitest';
import { cotaDaConta, resumoCota, type CotaContaResumo } from './cotaResumo';

const lida: CotaContaResumo = {
  id: 'claude:/home/x/.claude',
  estado: 'lida',
  janelas: [{ rotulo: '5h', pct: 42.4 }, { rotulo: '7d', pct: 17.6 }, { rotulo: 'Fable', pct: 100, por_modelo: true }],
};

describe('cotaDaConta', () => {
  it('casa pelo config dir com o prefixo claude:', () => {
    expect(cotaDaConta([lida], '/home/x/.claude')).toBe(lida);
    expect(cotaDaConta([lida], '/home/x/.claude-b')).toBeUndefined();
  });
});

describe('resumoCota', () => {
  it('uma janela por trecho, arredondada', () => {
    expect(resumoCota(lida)).toBe('5h 42% · 7d 18% · Fable 100%');
  });
  it('vazio sem leitura, sem conta ou sem janela', () => {
    expect(resumoCota(undefined)).toBe('');
    expect(resumoCota({ ...lida, estado: 'expirada' })).toBe('');
    expect(resumoCota({ ...lida, janelas: [] })).toBe('');
    expect(resumoCota({ ...lida, janelas: [{ rotulo: '5h', pct: NaN }] })).toBe('');
  });
});
