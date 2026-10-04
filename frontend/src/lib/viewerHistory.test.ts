// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { closeViewerHistory, openViewerHistory, waitForViewerHistory } from './viewerHistory';

const baseState = { cpDepth: 2, other: 'preservado' };
const baseUrl = 'http://localhost:3000/#/chat/test/session';
let back: ReturnType<typeof vi.spyOn>;

function pop(state: unknown = baseState, url = baseUrl) {
  history.replaceState(state, '', url);
  window.dispatchEvent(new PopStateEvent('popstate', { state }));
}

beforeEach(() => {
  history.replaceState(baseState, '', baseUrl);
  back = vi.spyOn(history, 'back').mockImplementation(() => {
    queueMicrotask(() => pop());
  });
});

afterEach(async () => {
  closeViewerHistory();
  await waitForViewerHistory();
  vi.restoreAllMocks();
});

describe('histórico do visor', () => {
  it('Voltar fecha o visor, preserva a rota e não volta uma segunda vez', () => {
    const close = vi.fn();
    openViewerHistory(close);
    expect(history.state).toMatchObject(baseState);
    expect(location.href).toBe(baseUrl);
    pop();
    expect(close).toHaveBeenCalledOnce();
    closeViewerHistory();
    expect(back).not.toHaveBeenCalled();
    expect(location.href).toBe(baseUrl);
  });

  it('fechar pelo botão ou Escape consome a entrada antes da próxima abertura', async () => {
    openViewerHistory(vi.fn());
    closeViewerHistory();
    expect(back).toHaveBeenCalledOnce();
    await waitForViewerHistory();
    expect(history.state).toEqual(baseState);
    const close = vi.fn();
    openViewerHistory(close);
    pop();
    expect(close).toHaveBeenCalledOnce();
    expect(back).toHaveBeenCalledOnce();
  });

  it('trocar a mídia aberta não empilha outra parada para Voltar', () => {
    const push = vi.spyOn(history, 'pushState');
    const oldClose = vi.fn();
    const currentClose = vi.fn();
    openViewerHistory(oldClose);
    openViewerHistory(currentClose);
    expect(push).toHaveBeenCalledOnce();
    pop();
    expect(oldClose).not.toHaveBeenCalled();
    expect(currentClose).toHaveBeenCalledOnce();
  });

  it('uma navegação para outra rota fecha o visor sem desfazer a navegação', () => {
    const close = vi.fn();
    openViewerHistory(close);
    pop(null, 'http://localhost:3000/#/');
    expect(close).toHaveBeenCalledOnce();
    closeViewerHistory();
    expect(back).not.toHaveBeenCalled();
    expect(location.hash).toBe('#/');
  });

  it('retira o listener depois de fechar, sem fechar uma mídia antiga de novo', async () => {
    const close = vi.fn();
    openViewerHistory(close);
    closeViewerHistory();
    await waitForViewerHistory();
    pop();
    expect(close).not.toHaveBeenCalled();
    expect(back).toHaveBeenCalledOnce();
  });
});
