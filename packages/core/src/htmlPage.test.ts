import { describe, expect, it } from 'vitest';
import { frameHeight, htmlPageFromResult, isHtmlRenderTool, pageFetchState, reservedHeight } from './htmlPage';

const ref = { id: 'a1', title: 'T', height: null, heights: { '360': 500, '728': 380, '1000': 360 } };

describe('htmlPage', () => {
  it('reconhece a tool do hangar e só ela', () => {
    expect(isHtmlRenderTool('mcp__hangar__html_render')).toBe(true);
    for (const n of ['hangar__html_render', 'hangar.html_render', 'hangar/html_render', 'mcp__hangar.html_render']) {
      expect(isHtmlRenderTool(n)).toBe(true);
    }
    expect(isHtmlRenderTool('mcp__outro__html_render')).toBe(false);
    expect(isHtmlRenderTool('mcp__xhangar__html_render')).toBe(false);
    expect(isHtmlRenderTool('hangar_html_render')).toBe(false);
    expect(isHtmlRenderTool('Read')).toBe(false);
  });

  it('lê a referência do resultado e ignora rascunho e erro', () => {
    const ok = JSON.stringify({ hangar_page: ref, message: 'x' });
    expect(htmlPageFromResult('mcp__hangar__html_render', ok)?.id).toBe('a1');
    expect(htmlPageFromResult('mcp__hangar__html_render', JSON.stringify({ draft: { id: 'b' } }))).toBeNull();
    expect(htmlPageFromResult('mcp__hangar__html_render', 'erro_pagina_invalida: title')).toBeNull();
    expect(htmlPageFromResult('Read', ok)).toBeNull();
  });

  it('lê a referência quando o resultado vem como lista de blocos de texto', () => {
    const text = JSON.stringify({ hangar_page: ref, message: 'x' });
    const blocks = JSON.stringify([{ type: 'text', text: text.slice(0, 10) }, { type: 'image' }, { type: 'text', text: text.slice(10) }]);
    expect(htmlPageFromResult('mcp__hangar__html_render', blocks)?.heights['728']).toBe(380);
    expect(htmlPageFromResult('mcp__hangar__html_render', JSON.stringify([{ type: 'text', text: 'erro' }]))).toBeNull();
  });

  it('lê o CallToolResult inteiro que o Codex grava, estruturado primeiro', () => {
    const text = JSON.stringify({ hangar_page: ref, message: 'x' });
    const structured = JSON.stringify({ content: [{ type: 'text', text: 'outro' }], structuredContent: { hangar_page: { ...ref, id: 's1' } } });
    expect(htmlPageFromResult('mcp__hangar__html_render', structured)?.id).toBe('s1');
    const onlyText = JSON.stringify({ content: [{ type: 'text', text: text.slice(0, 7) }, { type: 'text', text: text.slice(7) }] });
    expect(htmlPageFromResult('hangar.html_render', onlyText)?.id).toBe('a1');
    expect(htmlPageFromResult('mcp__hangar__html_render', JSON.stringify({ content: [{ type: 'text', text: 'erro' }], isError: true }))).toBeNull();
  });

  it('reserva a altura da largura medida mais próxima', () => {
    expect(reservedHeight(ref, 390)).toBe(500);
    expect(reservedHeight(ref, 700)).toBe(380);
    expect(reservedHeight({ ...ref, heights: {} }, 700)).toBe(240);
  });

  it('altura informada pela página vence, com teto do agente e limites', () => {
    expect(frameHeight(ref, 728, 420)).toBe(420);
    expect(frameHeight({ ...ref, height: 300 }, 728, 420)).toBe(300);
    expect(frameHeight(ref, 728, 5000)).toBe(2000);
    expect(frameHeight(ref, 728, 10)).toBe(80);
    expect(frameHeight(ref, 728, null)).toBe(380);
  });

  it('404 só é expirou com o código do servidor de páginas, rede é erro', () => {
    expect(pageFetchState(200)).toBe('ready');
    expect(pageFetchState(404, 'erro_pagina_expirou')).toBe('expired');
    expect(pageFetchState(404)).toBe('error');
    expect(pageFetchState(404, 'Not Found')).toBe('error');
    expect(pageFetchState(500)).toBe('error');
    expect(pageFetchState('network')).toBe('error');
  });
});
