<script lang="ts">
  import { isEmptyBand, type PluginNode as Node } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    /** Árvore do `AbovePrompt` como o Claude Code a desenhou, de todos os mods juntos. */
    tree: Node;
  }
  let { tree }: Props = $props();
</script>

{#if !isEmptyBand(tree)}
  <!-- Sem aria-live: mod com relógio muda a cada segundo, e o leitor de tela leria sem parar. -->
  <section class="plugin-band" aria-label={m.plugin_band_label()}>
    <PluginNode node={tree} />
  </section>
{/if}

<style>
  .plugin-band {
    position: relative;
    margin: 0 var(--space-3) var(--space-1);
    padding: var(--space-1) var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text);
    background: var(--surface-inset);
    border-radius: var(--radius-md);
    overflow-x: auto;
    overflow-y: hidden;
  }
</style>
