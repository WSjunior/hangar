// @vitest-environment happy-dom
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import Harness from './Composer.permission.harness.svelte';
import * as m from '../paraglide/messages';
import * as api from '@hangar/core';

vi.mock('@hangar/core', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@hangar/core')>()),
  getPermissionModes: vi.fn().mockResolvedValue({ current: 'plan', modes: [], sondavel: true, previous_non_plan: 'manual' }),
  setPermissionMode: vi.fn().mockResolvedValue({ mode: 'plan', current: 'plan', previous_non_plan: 'manual' }),
  getCommands: vi.fn().mockResolvedValue([]),
  setModelEffort: vi.fn(),
  uploadFile: vi.fn(),
  transcribeFile: vi.fn(),
  getCodexModels: vi.fn().mockResolvedValue([]),
  getPiModels: vi.fn().mockResolvedValue([]),
  // O prefetch do Composer (cache de catálogo) chama estes ao montar — sem eles no mock o
  // módulo nem sobe.
  getKimiModels: vi.fn().mockResolvedValue({ models: [], default: null }),
  getModelOptions: vi.fn().mockResolvedValue({ kind: 'claude', models: [] }),
  isTimeoutError: vi.fn(() => false),
  isAbortError: vi.fn(() => false),
}));

describe('Composer — pílula de permissão reage a sessionState', () => {
  beforeEach(() => vi.clearAllMocks());

  it('monta com working e rele ao virar idle', async () => {
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el }) as unknown as { setState: (s: string) => void };
    // espera o $effect inicial disparar
    await tick(); await tick(); await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();
    expect(vi.mocked(api.getPermissionModes)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.getPermissionModes)).toHaveBeenCalledWith('perm-test', false);

    // muda working -> idle: deve disparar segunda leitura
    (harness as unknown as { setState: (v: string) => void }).setState('idle');
    await tick(); await tick(); await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();

    expect(vi.mocked(api.getPermissionModes)).toHaveBeenCalledTimes(2);
    // segunda chamada também sem sondar
    expect(vi.mocked(api.getPermissionModes).mock.calls[1]).toEqual(['perm-test', false]);

    unmount(harness as never);
    document.body.innerHTML = '';
  });

  it('atualiza o controle quando o terminal troca de modo sem mudar o estado', async () => {
    vi.mocked(api.getPermissionModes).mockResolvedValue({
      current: 'auto', modes: ['plan', 'auto', 'manual'], sondavel: true, previous_non_plan: 'auto',
    });
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el }) as unknown as {
      setPermission: (mode: string, previous: string) => void;
    };
    await tick(); await new Promise((resolve) => setTimeout(resolve, 0)); await tick();

    harness.setPermission('plan', 'auto');
    await tick();

    const trigger = el.querySelector<HTMLButtonElement>('[data-mode-trigger]')!;
    expect(trigger.textContent?.trim()).toBe(m.chat_mode_plan());
    expect(trigger.getAttribute('aria-pressed')).toBe('true');

    unmount(harness as never);
    document.body.innerHTML = '';
  });

  it('resposta GET antiga não sobrescreve um modo mais novo confirmado pelo SSE', async () => {
    let resolveGet!: (value: {
      current: string; modes: string[]; sondavel: boolean; previous_non_plan: string;
    }) => void;
    vi.mocked(api.getPermissionModes).mockReturnValue(new Promise((resolve) => { resolveGet = resolve; }));
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el }) as unknown as {
      setPermission: (mode: string, previous: string) => void;
    };
    await tick(); await new Promise((resolve) => setTimeout(resolve, 0)); await tick();
    expect(api.getPermissionModes).toHaveBeenCalled();

    harness.setPermission('plan', 'auto');
    await tick();
    resolveGet({ current: 'auto', modes: ['plan', 'auto'], sondavel: true, previous_non_plan: 'auto' });
    await tick(); await new Promise((resolve) => setTimeout(resolve, 0)); await tick();

    const trigger = el.querySelector<HTMLButtonElement>('[data-mode-trigger]')!;
    expect(trigger.textContent?.trim()).toBe(m.chat_mode_plan());
    expect(trigger.getAttribute('aria-pressed')).toBe('true');

    unmount(harness as never);
    document.body.innerHTML = '';
  });

  it('Alt+Shift+P passa pro PRÓXIMO modo do ciclo vivo (e dá a volta no fim)', async () => {
    vi.mocked(api.getPermissionModes).mockResolvedValue({ current: 'acceptEdits', modes: ['plan', 'acceptEdits'], sondavel: true, previous_non_plan: 'acceptEdits' });
    vi.mocked(api.setPermissionMode).mockResolvedValue({ mode: 'plan', current: 'plan', previous_non_plan: 'acceptEdits' });
    // desktop: o atalho é só de tela larga
    window.matchMedia = ((q: string) => ({ matches: true, media: q, addEventListener() {}, removeEventListener() {} })) as never;
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el });
    await tick(); await tick(); await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();

    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyP', key: 'P', altKey: true, shiftKey: true, bubbles: true }));
    await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();
    // acceptEdits é o último da lista de 2 -> volta pro primeiro
    expect(vi.mocked(api.setPermissionMode)).toHaveBeenCalledWith('perm-test', 'plan');

    // sem Shift não é o atalho
    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyP', key: 'p', altKey: true, bubbles: true }));
    await tick(); await new Promise((r) => setTimeout(r, 0));
    expect(vi.mocked(api.setPermissionMode)).toHaveBeenCalledTimes(1);

    unmount(harness as never);
    document.body.innerHTML = '';
  });

  it.each(['Alt+Shift+P', 'Shift+Tab'])('sem ciclo em cache, %s sonda antes de aplicar o próximo modo', async (atalho) => {
    vi.mocked(api.getPermissionModes)
      .mockResolvedValueOnce({ current: 'plan', modes: [], sondavel: true, previous_non_plan: 'manual' })
      .mockResolvedValueOnce({ current: 'plan', modes: ['plan', 'auto'], sondavel: true, previous_non_plan: 'manual' });
    vi.mocked(api.setPermissionMode).mockResolvedValue({ mode: 'auto', current: 'auto', previous_non_plan: 'auto' });
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el });
    await tick(); await tick(); await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();

    if (atalho === 'Shift+Tab') {
      el.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true }));
    } else {
      document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyP', key: 'P', altKey: true, shiftKey: true, bubbles: true }));
    }
    await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();
    await new Promise((r) => setTimeout(r, 0)); await tick();
    expect(vi.mocked(api.getPermissionModes).mock.calls.at(-1)).toEqual(['perm-test', true]);
    expect(vi.mocked(api.setPermissionMode)).toHaveBeenCalledWith('perm-test', 'auto');

    unmount(harness as never);
    document.body.innerHTML = '';
  });

  it.each([
    ['auto', 'manual', 'acceptEdits', 'plan'],
    ['bypassPermissions', 'auto', 'manual', 'acceptEdits', 'plan'],
  ])('Shift+Tab percorre o ciclo disponível %j; Ctrl+L foca o campo', async (...modes) => {
    vi.mocked(api.getPermissionModes).mockResolvedValue({
      current: 'plan', modes, sondavel: true, previous_non_plan: 'manual',
    });
    vi.mocked(api.setPermissionMode).mockImplementation(async (_name, mode) => ({
      mode, current: mode, previous_non_plan: 'manual',
    }));
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el });
    await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();

    const ta = el.querySelector('textarea')!;
    ta.blur();
    document.dispatchEvent(new KeyboardEvent('keydown', { code: 'KeyL', key: 'l', ctrlKey: true, bubbles: true }));
    expect(document.activeElement).toBe(ta);

    for (const mode of [...modes, modes[0]]) {
      const ev = new KeyboardEvent('keydown', { key: 'Tab', code: 'Tab', shiftKey: true, bubbles: true, cancelable: true });
      ta.dispatchEvent(ev);
      expect(ev.defaultPrevented).toBe(true);
      await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();
      expect(api.setPermissionMode).toHaveBeenLastCalledWith('perm-test', mode);
      expect(el.querySelector('[data-mode-trigger]')?.getAttribute('aria-pressed')).toBe(String(mode === 'plan'));
    }
    expect(vi.mocked(api.setPermissionMode).mock.calls.map(([, mode]) => mode)).toEqual([...modes, modes[0]]);

    await unmount(harness as never);
    document.body.innerHTML = '';
  });

  it('Shift+Tab não aplica modos a uma sessão sem ciclo disponível', async () => {
    vi.mocked(api.getPermissionModes).mockResolvedValue({
      current: 'dontAsk', modes: [], sondavel: false, previous_non_plan: 'manual',
    });
    const el = document.createElement('div');
    document.body.appendChild(el);
    const harness = mount(Harness as never, { target: el });
    await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();

    el.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true }));
    await tick(); await new Promise((r) => setTimeout(r, 0)); await tick();
    expect(api.setPermissionMode).not.toHaveBeenCalled();
    expect(api.getPermissionModes).toHaveBeenCalledTimes(1);
    expect(el.textContent).toContain(m.permissao_sem_ciclo());

    await unmount(harness as never);
    document.body.innerHTML = '';
  });
});
