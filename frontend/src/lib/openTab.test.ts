// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { openInNewTab } from './openTab';

afterEach(() => vi.restoreAllMocks());

describe('openInNewTab', () => {
  it('janela aberta é sucesso, perde o opener e navega sem referrer', () => {
    const aberta = { opener: window, document: document.implementation.createHTMLDocument('') };
    // Como o navegador: com `noopener`/`noreferrer` nas features o retorno é sempre null.
    vi.spyOn(window, 'open').mockImplementation((_u, _t, features) =>
      /noopener|noreferrer/.test(String(features ?? '')) ? null : (aberta as unknown as Window));
    expect(openInNewTab('https://exemplo.dev/a?b=1')).toBe(true);
    expect(aberta.opener).toBeNull();
    const head = aberta.document.head;
    expect(head.querySelector('meta[name="referrer"]')?.getAttribute('content')).toBe('no-referrer');
    expect(head.querySelector('meta[http-equiv="refresh"]')?.getAttribute('content'))
      .toBe('0;url="https://exemplo.dev/a?b=1"');
  });

  it('janela bloqueada é falha', () => {
    vi.spyOn(window, 'open').mockReturnValue(null);
    expect(openInNewTab('https://exemplo.dev')).toBe(false);
  });
});
