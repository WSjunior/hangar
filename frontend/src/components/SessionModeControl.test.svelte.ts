// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount, type ComponentProps } from 'svelte';
import SessionModeControl from './SessionModeControl.svelte';
import * as m from '../paraglide/messages';

const mounted: ReturnType<typeof mount>[] = [];

afterEach(async () => {
  for (const component of mounted) await unmount(component);
  document.body.innerHTML = '';
});

async function render(props: ComponentProps<typeof SessionModeControl>) {
  const target = document.createElement('div');
  document.body.appendChild(target);
  mounted.push(mount(SessionModeControl, { target, props }));
  await tick();
  return target;
}

it('oferece Normal e Planejar para Codex e destaca Planejar', async () => {
  const onApply = vi.fn();
  const target = await render({ provider: 'codex', current: 'plan', onApply });
  const trigger = target.querySelector<HTMLButtonElement>('[data-mode-trigger]')!;
  expect(trigger.textContent).toContain(m.chat_mode_plan());
  expect(trigger.classList.contains('planning')).toBe(true);
  trigger.click();
  await tick();
  const normal = [...document.querySelectorAll<HTMLButtonElement>('[data-mode-option]')]
    .find((button) => button.textContent?.includes(m.chat_mode_normal()))!;
  normal.click();
  expect(onApply).toHaveBeenCalledWith('default');
});

it('identifica modos Claude indisponíveis no ciclo desta sessão', async () => {
  const target = await render({
    provider: 'claude', current: 'auto', modes: ['plan', 'auto', 'manual'], onApply: vi.fn(),
  });
  target.querySelector<HTMLButtonElement>('[data-mode-trigger]')!.click();
  await tick();
  const bypass = [...document.querySelectorAll<HTMLButtonElement>('[data-mode-option]')]
    .find((button) => button.dataset.mode === 'bypassPermissions')!;
  expect(bypass.disabled).toBe(true);
  expect(bypass.textContent).toContain(m.chat_mode_unavailable());
});
