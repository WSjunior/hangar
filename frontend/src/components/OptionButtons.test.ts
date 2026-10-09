// @vitest-environment happy-dom
import { describe, it, expect, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import OptionButtons from './OptionButtons.svelte';
import { overwriteGetLocale } from '../paraglide/runtime';

overwriteGetLocale(() => 'pt');

async function montar(question: string) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const comp = mount(OptionButtons, { target: el, props: { question, options: ['Concluí', 'Cancelar'], onSelect: vi.fn(), onCancel: vi.fn() } });
  await tick();
  return { el, fim: () => { unmount(comp); el.remove(); } };
}

describe('OptionButtons: links na pergunta', () => {
  it('URL vira link que abre fora do app', async () => {
    const { el, fim } = await montar('srv pede para abrir https://a.com/x: confirme');
    const a = el.querySelector('p.question a') as HTMLAnchorElement;
    expect(a.getAttribute('href')).toBe('https://a.com/x');
    expect(a.target).toBe('_blank');
    expect(a.rel).toContain('noopener');
    expect(el.querySelector('p.question')!.textContent).toBe('srv pede para abrir https://a.com/x: confirme');
    fim();
  });
  it('sem URL não há link; crases seguem em code', async () => {
    const { el, fim } = await montar('rode `curl https://a.com` agora');
    expect(el.querySelector('p.question a')).toBeNull();
    expect(el.querySelector('p.question code')!.textContent).toBe('curl https://a.com');
    fim();
  });
});
