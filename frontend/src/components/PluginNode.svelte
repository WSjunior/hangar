<script lang="ts">
  import { decodeRaster, textOf, type PluginElement, type PluginNode as Node, type RasterCell } from '@hangar/core';
  import { boxStyle, textStyle } from '../lib/pluginUiStyle';
  import { renderMarkdown } from '../lib/markdown';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    node: Node;
  }
  let { node }: Props = $props();

  const el = $derived(node && typeof node === 'object' ? (node as PluginElement) : null);
  const p = $derived((el?.props ?? {}) as Record<string, unknown>);
  const str = (v: unknown) => (typeof v === 'string' ? v : '');

  // Células vizinhas da mesma cor viram um trecho só: uma barra de 100 colunas não vira 100 spans.
  type Run = { text: string; fg: string | null; bg: string | null };
  function runs(row: RasterCell[]): Run[] {
    const out: Run[] = [];
    for (const c of row) {
      const last = out[out.length - 1];
      if (last && last.fg === c.fg && last.bg === c.bg) last.text += c.ch;
      else out.push({ text: c.ch, fg: c.fg, bg: c.bg });
    }
    return out;
  }
  const raster = $derived(
    el?.type === 'Raster' && typeof p.cells === 'string'
      ? decodeRaster(p.cells, Number(p.columns) || 0, Number(p.rows) || 0).map(runs)
      : [],
  );
  // O SVG vira imagem: dentro de <img> ele não roda script, só as animações de CSS dele.
  const svgSrc = $derived(
    el?.type === 'Svg' && typeof p.source === 'string'
      ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(p.source)}`
      : '',
  );
</script>

{#if typeof node === 'string' || typeof node === 'number'}{node}{:else if el}
  {#if el.type === 'Box'}
    <div class="box" style={boxStyle(p)}>
      {#each el.children ?? [] as child, i (i)}<PluginNode node={child} />{/each}
    </div>
  {:else if el.type === 'Text'}
    <span style={textStyle(p)}>{#each el.children ?? [] as child, i (i)}<PluginNode node={child} />{/each}</span>
  {:else if el.type === 'Raster'}
    <span class="raster">{#each raster as row, r (r)}<span class="raster-row">{#each row as run, i (i)}<span style:color={run.fg} style:background={run.bg}>{run.text}</span>{/each}</span>{/each}</span>
  {:else if el.type === 'Svg'}
    <img class="svg" src={svgSrc} alt={str(p.alt)} width={Number(p.width) || undefined} height={Number(p.height) || undefined} />
  {:else if el.type === 'Markdown'}
    <div class="md" class:dim={p.dimColor === true}>{@html renderMarkdown(str(p.text))}</div>
  {:else if el.type === 'Code'}
    <pre class="code">{str(p.source)}</pre>
  {:else if el.type === 'Link'}
    <a href={str(p.href)} target="_blank" rel="noopener noreferrer">{str(p.label) || textOf(el.children) || str(p.href)}</a>
  {:else if el.type === 'Button'}
    <!-- Botão de mod ainda não responde no Hangar: aparece como rótulo, sem fingir que clica. -->
    <span class="button" class:primary={p.variant === 'primary'}>{str(p.label) || textOf(el.children)}</span>
  {:else if el.type === 'Image'}
    <span class="alt">{str(p.alt)}</span>
  {:else}
    {#each el.children ?? [] as child, i (i)}<PluginNode node={child} />{/each}
  {/if}
{/if}

<style>
  .box { box-sizing: border-box; }
  .raster { display: inline-flex; flex-direction: column; white-space: pre; line-height: 1; }
  .raster-row { display: block; }
  .svg { display: block; max-width: 100%; height: auto; align-self: center; }
  .md :global(p) { margin: 0; }
  .md.dim { opacity: 0.6; }
  .code { margin: 0; white-space: pre-wrap; }
  .button { padding: 0 1ch; border-radius: var(--radius-sm); background: var(--surface-inset); }
  .button.primary { color: var(--accent); }
  .alt { opacity: 0.6; }
</style>
