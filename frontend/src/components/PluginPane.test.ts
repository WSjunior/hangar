// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import type { PluginPane as Pane } from '@hangar/core';
import PluginPane from './PluginPane.svelte';

const painel = (id: string, title: string, tree: Pane['tree'] = { type: 'Text', children: [`corpo ${id}`] }): Pane =>
  ({ id, title, placement: 'dock', columns: 58, tree });
const tres = [painel('pm-mock-pm', 'PM-1'), painel('pm-mock-mr', 'MR ●2'), painel('pm-mock-jenkins', 'Jenkins')];

let alvo: HTMLElement | null = null;
let comp: ReturnType<typeof mount> | null = null;
async function montar(props: Record<string, unknown>) {
  alvo = document.createElement('div');
  document.body.append(alvo);
  comp = mount(PluginPane, { target: alvo, props: props as never });
  await tick();
  return alvo;
}
afterEach(async () => {
  if (comp) await unmount(comp);
  comp = null;
  alvo?.remove();
});

describe('PluginPane com vários painéis', () => {
  it('vira abas: uma fileira com os títulos e só o painel ativo desenhado', async () => {
    const el = await montar({ pane: tres[1], tabs: tres, onPress: vi.fn(), onShow: vi.fn() });
    const abas = [...el.querySelectorAll('[role="tab"]')];
    expect(abas.map((a) => a.textContent)).toEqual(['PM-1', 'MR ●2', 'Jenkins']);
    expect(abas.map((a) => a.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false']);
    expect(el.textContent).toContain('corpo pm-mock-mr');
    expect(el.textContent).not.toContain('corpo pm-mock-pm');
  });

  it('clicar noutra aba pede a troca; clicar na ativa não pede nada', async () => {
    const onShow = vi.fn();
    const el = await montar({ pane: tres[1], tabs: tres, onPress: vi.fn(), onShow });
    const abas = el.querySelectorAll<HTMLButtonElement>('[role="tab"]');
    abas[2].click();
    abas[1].click();
    expect(onShow.mock.calls).toEqual([['pm-mock-jenkins']]);
  });

  it('um ✕ só, que fecha o painel ativo', async () => {
    const onClose = vi.fn();
    const el = await montar({ pane: tres[1], tabs: tres, onPress: vi.fn(), onClose, onShow: vi.fn() });
    const fechar = el.querySelectorAll<HTMLButtonElement>('button.close');
    expect(fechar).toHaveLength(1);
    fechar[0].click();
    expect(onClose).toHaveBeenCalledWith('pm-mock-mr');
  });

  it('com um painel só, o cabeçalho de sempre, sem fileira de abas', async () => {
    const el = await montar({ pane: tres[0], tabs: [tres[0]], onPress: vi.fn() });
    expect(el.querySelector('[role="tablist"]')).toBeNull();
    expect(el.querySelector('.title')?.textContent).toBe('PM-1');
  });

  it('painel só com o nó engine aparece como aba de corpo vazio', async () => {
    const vazio = painel('vitrine-vazio', 'Vazio', { type: 'engine', ref: 0 } as Pane['tree']);
    const el = await montar({ pane: vazio, tabs: [tres[0], vazio], onPress: vi.fn(), onShow: vi.fn() });
    expect([...el.querySelectorAll('[role="tab"]')].map((a) => a.textContent)).toEqual(['PM-1', 'Vazio']);
    expect(el.querySelector('.body')?.textContent?.trim()).toBe('');
  });
});
