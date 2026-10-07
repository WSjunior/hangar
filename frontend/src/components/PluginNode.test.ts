// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import type { PluginNode as Node } from '@hangar/core';
import PluginNode from './PluginNode.svelte';

const V28: Node = { type: 'Box', props: { key: 'V28-escopo', flexDirection: 'row', columnGap: 2, paddingX: 1, borderStyle: 'round' }, hover: { borderColor: '#5aa6ff' }, children: [
  { type: 'Text', hover: { color: '#3fb97a', bold: true }, children: ['texto que muda de cor'] },
  { type: 'Button', props: { key: 'V28-botao', label: 'Botão no escopo' }, press: { plugin: 'mod', handle: 1 }, hover: { color: '#e8a33d' } },
] };
const V29: Node = { type: 'Box', props: { key: 'V29-escopo', flexDirection: 'row' }, children: [
  { type: 'Text', children: ['passe o ponteiro'] },
  { type: 'Box', props: { position: 'absolute', top: 1, left: 2, display: 'none' }, hover: { display: 'flex' }, children: [{ type: 'Text', children: ['cartão'] }] },
] };

// O happy-dom serializa o style com espaço depois dos dois-pontos; as asserções comparam sem ele.
const estilo = (e: Element | null) => (e?.getAttribute('style') ?? '').replace(/: /g, ':');

let alvo: HTMLElement | null = null;
let comp: ReturnType<typeof mount> | null = null;
async function montar(props: Record<string, unknown>) {
  alvo = document.createElement('div');
  document.body.append(alvo);
  comp = mount(PluginNode, { target: alvo, props: props as never });
  await tick();
  return alvo;
}
afterEach(async () => {
  if (comp) await unmount(comp);
  comp = null;
  alvo?.remove();
});

describe('hover nos mods', () => {
  it('cartão absolute com display none aparece com o ponteiro no escopo e some ao sair', async () => {
    const el = await montar({ node: V29 });
    const [escopo, cartao] = [...el.querySelectorAll<HTMLElement>('.box')];
    expect(estilo(cartao)).toContain('display:none');
    escopo.dispatchEvent(new Event('pointerenter'));
    await tick();
    expect(estilo(cartao)).toContain('display:flex');
    expect(estilo(cartao)).toContain('position:absolute');
    escopo.dispatchEvent(new Event('pointerleave'));
    await tick();
    expect(estilo(cartao)).toContain('display:none');
  });

  it('o hover muda a borda do escopo, o texto e o rótulo do botão (V28)', async () => {
    const el = await montar({ node: V28, onPress: vi.fn() });
    const escopo = el.querySelector<HTMLElement>('.box')!;
    escopo.dispatchEvent(new Event('pointerenter'));
    await tick();
    expect(estilo(escopo)).toContain('border:1px solid #5aa6ff');
    expect(estilo(el.querySelector('span'))).toContain('color:#3fb97a');
    expect(estilo(el.querySelector('button'))).toContain('color:#e8a33d');
  });

  it('o Button aplica o conjunto inteiro de estilo do hover, no botão e no rótulo sem onPress', async () => {
    const cheio: Node = { type: 'Box', props: { key: 'k' }, children: [
      { type: 'Button', props: { key: 'b', label: 'x' }, press: { plugin: 'mod', handle: 1 }, hover: { color: '#e8a33d', backgroundColor: '#112233', italic: true, underline: true, strikethrough: true, bold: true, dimColor: true } },
    ] };
    for (const onPress of [vi.fn(), undefined]) {
      const el = await montar({ node: cheio, onPress });
      el.querySelector<HTMLElement>('.box')!.dispatchEvent(new Event('pointerenter'));
      await tick();
      const s = estilo(el.querySelector('.button'));
      for (const parte of ['color:#e8a33d', 'background:#112233', 'font-style:italic', 'text-decoration:underline line-through', 'font-weight:700', 'opacity:0.6']) expect(s).toContain(parte);
      await unmount(comp!);
      comp = null;
      alvo!.remove();
    }
  });

  it('hover fora de um Box com key não acende', async () => {
    const solto: Node = { type: 'Box', props: {}, children: [{ type: 'Text', hover: { color: '#ff0000' }, children: ['x'] }] };
    const el = await montar({ node: solto });
    el.querySelector<HTMLElement>('.box')!.dispatchEvent(new Event('pointerenter'));
    await tick();
    expect(estilo(el.querySelector('span'))).not.toContain('#ff0000');
  });
});

// A linha da faixa do mod `pm-mock`: dois botões `plain` e um texto cortado com trechos coloridos dentro.
const LINHA_PM: Node = { type: 'Box', props: { key: 'pm-linha', flexDirection: 'row', width: 150 }, children: [
  { type: 'Button', props: { key: 'pm-abrir', plain: true, label: '▸ TAREFA-123' }, press: { plugin: 'pm-mock', handle: 1 } },
  { type: 'Text', children: [' '] },
  { type: 'Button', props: { key: 'pm-fechar', plain: true, dimColor: true, label: 'fechar' }, press: { plugin: 'pm-mock', handle: 1 } },
  { type: 'Text', props: { wrap: 'truncate-end' }, children: [
    { type: 'Text', props: { dimColor: true }, children: [' · '] },
    { type: 'Text', props: { color: '#3fb97a', wrap: 'wrap' }, children: ['servico_exemplo ● 2 threads abertas'] },
  ] },
] };

describe('corte e quebra como no terminal (W6)', () => {
  it('o corte do Text de fora vale para os Text de dentro: eles herdam o white-space e não declaram o deles', async () => {
    const el = await montar({ node: LINHA_PM, onPress: vi.fn() });
    const [, cortado, ponto, colorido] = [...el.querySelectorAll('span')];
    for (const parte of ['white-space:pre', 'overflow:hidden', 'text-overflow:ellipsis', 'min-width:0']) expect(estilo(cortado)).toContain(parte);
    for (const filho of [ponto, colorido]) expect(estilo(filho)).not.toMatch(/white-space|overflow|min-width/);
    expect(estilo(colorido)).toContain('color:#3fb97a');
    expect(getComputedStyle(colorido).whiteSpace).toBe('pre');
  });

  it('o Button não encolhe na linha nem estica na coluna, e o rótulo não quebra no meio', async () => {
    const el = await montar({ node: LINHA_PM, onPress: vi.fn() });
    for (const botao of el.querySelectorAll('button')) {
      const s = getComputedStyle(botao);
      expect(s.flexShrink).toBe('0');
      expect(s.alignSelf).toBe('flex-start');
      expect(s.whiteSpace).toBe('pre-wrap');
      expect(s.maxWidth).toBe('100%');
    }
  });
});
