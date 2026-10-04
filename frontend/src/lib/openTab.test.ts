// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { openInNewTab } from './openTab';

afterEach(() => vi.restoreAllMocks());

describe('openInNewTab', () => {
  it('janela aberta é sucesso e perde o opener', () => {
    const aberta = { opener: window } as unknown as Window;
    // Como o navegador: com `noopener` nas features o retorno é sempre null.
    const open = vi.spyOn(window, 'open').mockImplementation((_u, _t, features) =>
      String(features ?? '').includes('noopener') ? null : aberta);
    expect(openInNewTab('https://exemplo.dev')).toBe(true);
    expect(open).toHaveBeenCalledWith('https://exemplo.dev', '_blank');
    expect(aberta.opener).toBeNull();
  });

  it('janela bloqueada é falha', () => {
    vi.spyOn(window, 'open').mockReturnValue(null);
    expect(openInNewTab('https://exemplo.dev')).toBe(false);
  });
});
