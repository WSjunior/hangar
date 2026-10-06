import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import * as m from '../paraglide/messages';

function fakeStorage(initial: Record<string, string> = {}) {
  const store = new Map<string, string>(Object.entries(initial));
  return {
    store,
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, v); },
    removeItem: (k: string) => { store.delete(k); },
  };
}

// A primeira importação puxa o @hangar/core e o Paraglide e, com a suíte inteira em paralelo, passa dos 5 s de um
// caso; aquece aqui, com prazo próprio, para o tempo do primeiro caso ser só o dele.
// O resetModules de cada caso reimporta o grafo inteiro, então com a máquina carregada os casos seguintes também
// passam dos 5 s: o prazo do arquivo cobre isso.
vi.setConfig({ testTimeout: 30_000 });

beforeAll(async () => {
  vi.stubGlobal('localStorage', fakeStorage());
  await import('./modsCelular.svelte');
}, 60_000);

beforeEach(() => {
  vi.resetModules();
  vi.unstubAllGlobals();
});

describe('preferência do aparelho', () => {
  it('sem chave salva, a interface dos mods vem oculta no celular e aparece no desktop', async () => {
    vi.stubGlobal('localStorage', fakeStorage());
    const { modsCelular, modsNaTela } = await import('./modsCelular.svelte');
    expect(modsCelular.ligado).toBe(false);
    expect(modsNaTela(false, modsCelular.ligado)).toBe(false);
    expect(modsNaTela(true, modsCelular.ligado)).toBe(true);
  });

  it("'1' salvo liga; '0' salvo desliga", async () => {
    vi.stubGlobal('localStorage', fakeStorage({ cp_mods_celular: '1' }));
    expect((await import('./modsCelular.svelte')).modsCelular.ligado).toBe(true);
    vi.resetModules();
    vi.stubGlobal('localStorage', fakeStorage({ cp_mods_celular: '0' }));
    expect((await import('./modsCelular.svelte')).modsCelular.ligado).toBe(false);
  });

  it('ligar e desligar grava a chave e muda na hora, sem reload', async () => {
    const storage = fakeStorage();
    vi.stubGlobal('localStorage', storage);
    const { modsCelular } = await import('./modsCelular.svelte');
    modsCelular.ligado = true;
    expect([modsCelular.ligado, storage.store.get('cp_mods_celular')]).toEqual([true, '1']);
    modsCelular.ligado = false;
    expect([modsCelular.ligado, storage.store.get('cp_mods_celular')]).toEqual([false, '0']);
  });

  it('storage bloqueado (modo privado): oculta e não quebra ao gravar', async () => {
    const erro = () => { throw new Error('bloqueado'); };
    vi.stubGlobal('localStorage', { getItem: erro, setItem: erro, removeItem: erro });
    const { modsCelular } = await import('./modsCelular.svelte');
    expect(modsCelular.ligado).toBe(false);
    modsCelular.ligado = true;
    expect(modsCelular.ligado).toBe(true);
  });
});

describe('item do menu "⋯"', () => {
  it('sessão sem mod nenhum (faixa vazia ou só o nó engine, sem painel): sem item', async () => {
    vi.stubGlobal('localStorage', fakeStorage());
    const { itemModsCelular } = await import('./modsCelular.svelte');
    expect(itemModsCelular(false, null, 0)).toBeNull();
    expect(itemModsCelular(false, { type: 'engine', ref: 1 } as never, 0)).toBeNull();
  });

  it('com faixa ou painel, no celular: item com a contagem de painéis; no desktop, nunca', async () => {
    vi.stubGlobal('localStorage', fakeStorage());
    const { itemModsCelular } = await import('./modsCelular.svelte');
    expect(itemModsCelular(false, { type: 'Box', children: [] }, 0)).toEqual({ paineis: 0 });
    expect(itemModsCelular(false, null, 3)).toEqual({ paineis: 3 });
    expect(itemModsCelular(true, { type: 'Box', children: [] }, 3)).toBeNull();
  });

  it('rótulo é o destino da troca, com a contagem quando oculto', async () => {
    vi.stubGlobal('localStorage', fakeStorage());
    const { rotuloModsCelular } = await import('./modsCelular.svelte');
    expect(rotuloModsCelular(true, 3)).toBe(m.mods_celular_ocultar());
    expect(rotuloModsCelular(false, 0)).toBe(m.mods_celular_mostrar());
    expect(rotuloModsCelular(false, 1)).toBe(m.mods_celular_mostrar_um());
    expect(rotuloModsCelular(false, 2)).toBe(m.mods_celular_mostrar_paineis({ n: '2' }));
    expect(m.mods_celular_mostrar_paineis({ n: '2' })).toContain('2');
  });
});
