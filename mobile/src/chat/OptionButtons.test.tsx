// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { Linking } from 'react-native';
import { describe, expect, it, vi } from 'vitest';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('../paraglide/messages', () => ({ comum_cancelar: () => 'cancelar', permissao_pedido: () => 'perm', comum_falha_envio_opcao: () => 'falha', opcoes_enviar_marcadas: () => 'enviar' }));
vi.mock('../stores/aparencia', () => ({ useAparencia: () => 'amber' }));
vi.mock('../ui/Icon', () => ({ Icon: () => null }));

import { OptionButtons } from './OptionButtons';

async function render(question: string) {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(createElement(OptionButtons, { question, options: ['Concluí', 'Cancelar'], onSelect: vi.fn(), onCancel: vi.fn() })));
  return { container, root };
}

describe('OptionButtons: links na pergunta', () => {
  it('URL na pergunta abre fora do app com Linking.openURL', async () => {
    const open = vi.spyOn(Linking, 'openURL');
    const { container, root } = await render('srv pede para abrir https://a.com/x: confirme');
    const link = container.querySelector('[role="link"]') as HTMLElement;
    expect(link.textContent).toBe('https://a.com/x');
    await act(async () => link.click());
    expect(open).toHaveBeenCalledWith('https://a.com/x');
    act(() => root.unmount());
  });

  it('sem URL não há link', async () => {
    const { container, root } = await render('Aprovar o plano?');
    expect(container.querySelector('[role="link"]')).toBeNull();
    act(() => root.unmount());
  });
});
