// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { expect, it, vi } from 'vitest';
import { CodexContextControl } from './CodexContextControl';

vi.mock('react-native', async (original) => ({
  ...await original<typeof import('react-native')>(),
  Switch: ({ value, disabled, onValueChange }: { value: boolean; disabled: boolean; onValueChange: (value: boolean) => void }) =>
    createElement('button', { role: 'switch', 'aria-checked': value, disabled, onClick: () => onValueChange(!value) }),
}));

it('lê o padrão e confirma a gravação antes de liberar a criação', async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement('div');
  const root = createRoot(container);
  const busy = vi.fn();
  let finish: (response: Response) => void = () => {};
  const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(async (_url, init) => init?.method === 'POST'
    ? new Promise<Response>(resolve => { finish = resolve; })
    : new Response(JSON.stringify({ contexto_estendido: true })));
  try {
    await act(async () => root.render(createElement(CodexContextControl, {
      server: { id: 'a', label: 'A', baseUrl: 'http://a.local', token: 'teste' }, onBusy: busy,
    })));
    const toggle = container.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute('aria-checked')).toBe('true');
    await act(async () => toggle.click());
    expect(toggle.disabled).toBe(true);
    expect(busy).toHaveBeenLastCalledWith(true);
    expect(fetchMock).toHaveBeenLastCalledWith('http://a.local/api/harness/codex/opcoes', expect.objectContaining({
      method: 'POST', body: JSON.stringify({ contexto_estendido: false }),
    }));
    await act(async () => finish(new Response(JSON.stringify({ contexto_estendido: false }))));
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    expect(busy).toHaveBeenLastCalledWith(false);
  } finally {
    await act(async () => root.unmount());
    fetchMock.mockRestore();
  }
});
