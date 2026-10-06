<script lang="ts">
  import { buttonKey, decodeRaster, hoverProps, inputKey, isHoverScope, safeHref, textOf, type PluginElement, type PluginInputKind, type PluginNode as Node, type RasterCell } from '@hangar/core';
  import { boxStyle, buttonStyle, textStyle } from '../lib/pluginUiStyle';
  import { renderMarkdown } from '../lib/markdown';
  import PluginInput from './PluginInput.svelte';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    node: Node;
    /** Clique num botão de mod, pela `key` dele; sem ele, os botões são só rótulo. */
    onPress?: (key: string) => void;
    /** Digitação num `Input`, só sem terminal; sem ele, o campo fica desabilitado com a dica. */
    onInput?: (key: string, kind: PluginInputKind, value: string) => void;
    /** Largura do lugar em colunas (faixa ou painel): `width` que a alcança vira 100%. */
    place?: number | null;
    /** O escopo de hover mais próximo (Box com `key`) está com o ponteiro em cima. */
    hoverOn?: boolean;
  }
  let { node, onPress, onInput, place = null, hoverOn = false }: Props = $props();

  const el = $derived(node && typeof node === 'object' ? (node as PluginElement) : null);
  // Box com `key` é escopo: acende com o ponteiro nele, e os filhos herdam. Os outros nós seguem o escopo de cima.
  let over = $state(false);
  const scope = $derived(el ? isHoverScope(el) : false);
  const lit = $derived(scope ? over : hoverOn);
  const p = $derived(el ? hoverProps(el, lit) : {});
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
  const rasterRows = (props: Record<string, unknown>) =>
    typeof props.cells === 'string'
      ? decodeRaster(props.cells, Number(props.columns) || 0, Number(props.rows) || 0).map(runs)
      : [];
  // O SVG vira imagem: dentro de <img> ele não roda script, só as animações de CSS dele.
  const svgSrc = (props: Record<string, unknown>) =>
    typeof props.source === 'string' ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(props.source)}` : '';
</script>

{#snippet kids(list: Node[] | undefined)}
  {#each list ?? [] as child, i (i)}<PluginNode node={child} {onPress} {onInput} {place} hoverOn={lit} />{/each}
{/snippet}

{#if typeof node === 'string' || typeof node === 'number'}{node}{:else if el}
  {#if el.type === 'Box'}
    <!-- svelte-ignore a11y_no_static_element_interactions (o hover só muda o desenho, não age) -->
    <div class="box" style={boxStyle(p, place)}
         onpointerenter={scope ? () => (over = true) : undefined}
         onpointerleave={scope ? () => (over = false) : undefined}>
      {@render kids(el.children)}
    </div>
  {:else if el.type === 'Text'}
    <span style={textStyle(p)}>{@render kids(el.children)}</span>
  {:else if el.type === 'Raster'}
    {@const raster = rasterRows(p)}
    <!-- O Raster vem com a largura do pane do terminal: em coluna mais estreita cada trecho encolhe na
         proporção das suas células, em vez de a faixa rolar de lado. -->
    <span class="raster" style:max-width="{Number(p.columns) || 0}ch">{#each raster as row, r (r)}<span class="raster-row">{#each row as run, i (i)}<span class="run" style:flex-grow={run.text.length} style:color={run.fg} style:background={run.bg}>{run.text}</span>{/each}</span>{/each}</span>
  {:else if el.type === 'Svg'}
    <img class="svg" src={svgSrc(p)} alt={str(p.alt)} width={Number(p.width) || undefined} height={Number(p.height) || undefined} />
  {:else if el.type === 'Markdown'}
    <div class="md" class:dim={p.dimColor === true}>{@html renderMarkdown(str(p.text))}</div>
  {:else if el.type === 'Code'}
    <pre class="code">{str(p.source)}</pre>
  {:else if el.type === 'Link'}
    {@const label = str(p.label) || textOf(el.children) || str(p.href)}
    {#if safeHref(p.href)}
      <a href={safeHref(p.href)} target="_blank" rel="noopener noreferrer">{label}</a>
    {:else}
      <span>{label}</span>
    {/if}
  {:else if el.type === 'Button'}
    {@const key = buttonKey(el)}
    {@const label = str(p.label) || textOf(el.children)}
    {#if onPress && key}
      <button type="button" class="button" class:plain={p.plain === true} class:primary={p.variant === 'primary'}
              class:dim={p.dimColor === true} style={buttonStyle(p)} onclick={() => onPress(key)}>{label}</button>
    {:else}
      <span class="button" class:plain={p.plain === true} class:primary={p.variant === 'primary'}
            class:dim={p.dimColor === true} style={buttonStyle(p)}>{label}</span>
    {/if}
  {:else if el.type === 'Input'}
    {@const key = inputKey(el)}
    <!-- `frame` é o nó: cada evento traz uma árvore nova, e o campo sabe que o mod desenhou de novo. -->
    <PluginInput label={str(p.label)} placeholder={str(p.placeholder)} value={str(p.value)} submitLabel={str(p.submitLabel)}
                 frame={el}
                 onInput={onInput && key ? (kind, value) => onInput(key, kind, value) : undefined} />
  {:else if el.type === 'Image'}
    <span class="alt">{str(p.alt)}</span>
  {:else}
    {@render kids(el.children)}
  {/if}
{/if}

<style>
  /* Âncora do `position: absolute` dos filhos, como o Box do Ink. */
  .box { box-sizing: border-box; position: relative; }
  .raster { display: flex; flex-direction: column; flex: 1 1 0; min-width: 0; align-self: center; white-space: pre; line-height: 1; }
  .raster-row { display: flex; min-width: 0; }
  .run { flex-basis: 0; flex-shrink: 1; min-width: 0; overflow: hidden; }
  .svg { display: block; max-width: 100%; height: auto; align-self: center; }
  .md :global(p) { margin: 0; }
  .md.dim { opacity: 0.6; }
  .code { margin: 0; white-space: pre-wrap; }
  .button { font: inherit; color: inherit; border: 0; padding: 0 1ch; border-radius: var(--radius-sm); background: var(--surface-inset); }
  /* O botão do app tem 44 px de área de toque e centraliza o texto; aqui ele é uma célula do
     terminal, na mesma linha dos textos ao lado. */
  button.button { cursor: pointer; min-height: 0; min-width: 0; display: inline-block; line-height: inherit; text-align: inherit; }
  button.button:hover { background: var(--surface-raised); }
  /* `plain` sai como no terminal: texto, sem pílula. */
  .button.plain { padding: 0; background: transparent; }
  button.button.plain:hover { text-decoration: underline; background: transparent; }
  .button.dim { opacity: 0.6; }
  .button.primary { color: var(--accent); }
  .alt { opacity: 0.6; }
</style>
