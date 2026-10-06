// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, tick, unmount } from 'svelte';
import type { PluginNode as Node } from '@hangar/core';
import * as m from '../paraglide/messages';
import PluginBand from './PluginBand.svelte';
import PluginInput from './PluginInput.svelte';

let alvo: HTMLElement | null = null;
let comp: ReturnType<typeof mount> | null = null;
afterEach(async () => {
  if (comp) await unmount(comp);
  comp = null;
  alvo?.remove();
});
function montar(props: Record<string, unknown>) {
  alvo = document.createElement('div');
  document.body.append(alvo);
  comp = mount(PluginInput, { target: alvo, props: props as never });
  flushSync();
  return alvo;
}

describe('Input de mod', () => {
  it('sem terminal: digitar manda change; Enter e o rótulo de envio mandam submit', async () => {
    const onInput = vi.fn();
    const el = montar({ label: 'V18 campo', placeholder: 'digite', value: '', submitLabel: 'ecoar', onInput });
    const campo = el.querySelector('input')!;
    expect(campo.disabled).toBe(false);
    expect(campo.placeholder).toBe('digite');
    campo.value = 'oi';
    campo.dispatchEvent(new Event('input', { bubbles: true }));
    campo.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    el.querySelector<HTMLButtonElement>('button.submit')!.click();
    await tick();
    expect(onInput.mock.calls).toEqual([['change', 'oi'], ['submit', 'oi'], ['submit', 'oi']]);
    expect(el.querySelector('button.submit')!.textContent).toBe('ecoar');
  });

  it('digitação igual ao valor desenhado também vai ao servidor (nada é engolido)', () => {
    const onInput = vi.fn();
    const el = montar({ label: '', placeholder: '', value: 'a', submitLabel: '', onInput });
    const campo = el.querySelector('input')!;
    expect(campo.value).toBe('a');
    campo.value = 'a';
    campo.dispatchEvent(new Event('input', { bubbles: true }));
    expect(onInput.mock.calls).toEqual([['change', 'a']]);
  });

  it('não usa a classe global `.field` do app, que empilha rótulo, campo e envio em coluna', () => {
    const el = montar({ label: 'V18 campo', placeholder: '', value: '', submitLabel: '', onInput: vi.fn() });
    expect(el.querySelector('.field')).toBeNull();
    const linha = el.querySelector('.plugin-field')!;
    expect([...linha.children].map((c) => c.tagName)).toEqual(['SPAN', 'INPUT', 'BUTTON']);
  });

  it('sem submitLabel, o rótulo de envio é o do app', () => {
    const el = montar({ label: '', placeholder: '', value: '', submitLabel: '', onInput: vi.fn() });
    expect(el.querySelector('button.submit')!.textContent).toBe(m.plugin_input_enviar());
  });

  it('com terminal (sem onInput): desabilitado, com o valor e a dica de digitar no terminal', () => {
    const el = montar({ label: 'V18 campo', placeholder: 'digite', value: 'abc', submitLabel: 'ecoar' });
    const campo = el.querySelector('input')!;
    expect(campo.disabled).toBe(true);
    expect(campo.value).toBe('abc');
    expect(el.querySelector('button.submit')).toBeNull();
    expect(el.textContent).toContain(m.plugin_input_no_terminal());
  });

  it('redesenho com valor novo não apaga o que a pessoa está digitando', () => {
    const props = $state({ label: '', placeholder: '', value: 'a', submitLabel: '', onInput: vi.fn() });
    alvo = document.createElement('div');
    document.body.append(alvo);
    comp = mount(PluginInput, { target: alvo, props });
    flushSync();
    const campo = alvo.querySelector('input')!;
    campo.focus();
    campo.value = 'abc';
    props.value = 'ab';
    flushSync();
    expect(campo.value).toBe('abc');
    campo.blur();
    props.value = 'abcd';
    flushSync();
    expect(campo.value).toBe('abcd');
  });

  // Cada evento `plugin_ui` traz uma árvore nova: `frame` muda mesmo quando o `value` desenhado é o mesmo.
  function montarVivo(value: string) {
    const onInput = vi.fn();
    const props = $state({ label: '', placeholder: '', value, submitLabel: '', onInput, frame: {} as object });
    alvo = document.createElement('div');
    document.body.append(alvo);
    comp = mount(PluginInput, { target: alvo, props });
    flushSync();
    const campo = alvo.querySelector('input')!;
    const redesenho = (v: string) => { props.value = v; props.frame = {}; flushSync(); };
    const digitar = (texto: string) => { campo.value = texto; campo.dispatchEvent(new Event('input', { bubbles: true })); };
    const enter = () => campo.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    return { campo, redesenho, digitar, enter, onInput };
  }

  it('redesenho em foco fica pendente e entra quando o campo perde o foco', () => {
    const { campo, redesenho, digitar } = montarVivo('');
    campo.focus();
    digitar('abc');
    redesenho('ab');
    expect(campo.value).toBe('abc');
    campo.blur();
    flushSync();
    expect(campo.value).toBe('ab');
  });

  it('o redesenho logo depois do próprio envio entra mesmo com foco, ainda que o valor seja o mesmo de antes', () => {
    const { campo, redesenho, digitar, enter, onInput } = montarVivo('');
    campo.focus();
    digitar('abc');
    enter();
    expect(onInput).toHaveBeenLastCalledWith('submit', 'abc');
    // O eco da última tecla, igual ao que se vê, não gasta a vez da resposta ao envio.
    redesenho('abc');
    expect(campo.value).toBe('abc');
    // O mod limpa o campo: o valor desenhado volta a ser vazio, como antes da digitação.
    redesenho('');
    expect(campo.value).toBe('');
    expect(document.activeElement).toBe(campo);
    // Só a resposta ao envio: o seguinte, em foco, volta a esperar.
    digitar('x');
    redesenho('y');
    expect(campo.value).toBe('x');
  });

  it('na faixa, a árvore nova do evento seguinte ao envio limpa o campo mesmo com o mesmo valor desenhado', () => {
    const arvore = (): Node => ({ type: 'Box', children: [
      { type: 'Text', children: ['campo'] },
      { type: 'Input', props: { key: 'E-campo', value: '', placeholder: 'digite' } }] });
    const onInput = vi.fn();
    const props = $state({ tree: arvore(), onInput });
    alvo = document.createElement('div');
    document.body.append(alvo);
    comp = mount(PluginBand, { target: alvo, props });
    flushSync();
    const campo = alvo.querySelector('input')!;
    campo.focus();
    campo.value = 'abc';
    campo.dispatchEvent(new Event('input', { bubbles: true }));
    campo.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(onInput).toHaveBeenLastCalledWith('above-prompt', 'E-campo', 'submit', 'abc');
    props.tree = arvore();
    flushSync();
    expect(campo.value).toBe('');
  });

  it('digitar de novo depois do envio fecha a vez: o redesenho seguinte não apaga o texto', () => {
    const { campo, redesenho, digitar, enter } = montarVivo('');
    campo.focus();
    digitar('abc');
    enter();
    digitar('abcd');
    redesenho('');
    expect(campo.value).toBe('abcd');
  });
});
