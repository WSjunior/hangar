// @vitest-environment happy-dom
// `escapes`: uma pergunta que veio de um SELETOR DE TUI (Codex sem thread, picker do pane) só tem
// as opções — "Digitar resposta" e "Conversar sobre isso" mandariam texto por um caminho que
// aquela sessão não tem. A primeira versão desligava as saídas com `{#if !textOpen && escapes}`,
// e o `{:else}` desenhava justamente o campo de texto: o oposto do pedido, e sem teste nenhum
// dizendo isso.
import { describe, it, expect, vi } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import AskQuestionStepper from './AskQuestionStepper.svelte';
import type { AskQuestionPayload } from '../lib/types';
import { overwriteGetLocale } from '../paraglide/runtime';

overwriteGetLocale(() => 'pt');

const payload: AskQuestionPayload = {
  questions: [{
    header: 'Codex',
    question: 'Confia nos hooks?',
    multiSelect: false,
    options: [{ label: 'Revisar', description: '' }, { label: 'Confiar', description: '' }],
  }],
};

function montar(escapes: boolean) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  const comp = mount(AskQuestionStepper, {
    target: el,
    props: { open: true, payload, onSubmit: vi.fn(), onClose: vi.fn(), escapes },
  });
  return { el, comp };
}

describe('AskQuestionStepper: saídas de texto', () => {
  it('responde várias perguntas do Codex pelos ids e mantém o erro para tentar de novo', async () => {
    const el = document.createElement('div');
    document.body.appendChild(el);
    const onSubmit = vi.fn().mockRejectedValueOnce(new Error('Conexão interrompida')).mockResolvedValueOnce(undefined);
    const comp = mount(AskQuestionStepper, { target: el, props: {
      open: true, onSubmit, onClose: vi.fn(), payload: {
        provider: 'codex', request_id: 0, questions: [
          { id: 'cor', header: 'Cor', question: 'Qual cor?', options: [{ label: 'Azul', description: '' }], multiSelect: false },
          { id: 'nome', header: 'Nome', question: 'Qual nome?', options: [], multiSelect: false },
        ],
      },
    } });
    await tick();
    el.querySelector<HTMLButtonElement>('.option-btn')!.click();
    await tick();
    const input = el.querySelector<HTMLInputElement>('input')!;
    expect(input).not.toBeNull();
    input.value = 'João';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await tick();
    el.querySelector<HTMLButtonElement>('.text-actions .primary-btn')!.click();
    await tick();
    el.querySelector<HTMLButtonElement>('.primary-btn')!.click();
    await tick();
    expect(onSubmit).toHaveBeenCalledWith([
      expect.objectContaining({ question_id: 'cor', kind: 'option', labels: ['Azul'] }),
      expect.objectContaining({ question_id: 'nome', kind: 'text', value: 'João' }),
    ]);
    await vi.waitFor(() => expect(el.textContent).toContain('Conexão interrompida'));
    expect(el.textContent).toContain('João');
    el.querySelector<HTMLButtonElement>('.primary-btn')!.click();
    await tick();
    expect(onSubmit).toHaveBeenCalledTimes(2);
    await unmount(comp);
    el.remove();
  });
  it('escapes=false não desenha campo de texto nem as saídas', () => {
    const { el, comp } = montar(false);
    expect(el.querySelector('input[type="text"]')).toBeNull();
    expect(el.querySelectorAll('.option-btn').length).toBe(2);   // as opções continuam
    unmount(comp);
  });

  it('escapes=true (o padrão do AskUserQuestion) mantém as duas saídas', () => {
    const { el, comp } = montar(true);
    expect(el.querySelectorAll('.escapes .ghost-btn').length).toBe(2);
    expect(el.querySelector('input[type="text"]')).toBeNull();   // só depois de clicar em digitar
    unmount(comp);
  });
});
