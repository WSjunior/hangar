<script lang="ts">
  import { BAND_SITE, isEmptyBand, type PluginNode as Node } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    /** Árvore do `AbovePrompt` como o Claude Code a desenhou, de todos os mods juntos. */
    tree: Node;
    /** Clique num botão de mod; sem ele, os botões são só rótulo. */
    onPress?: (site: string, key: string) => void;
    /** Resultado do último clique ("Copiado", o erro, ou o link que o navegador bloqueou); some sozinho. */
    notice?: { text: string; error: boolean; href?: string } | null;
  }
  let { tree, onPress, notice = null }: Props = $props();
</script>

{#if !isEmptyBand(tree) || notice}
  <!-- Sem aria-live na faixa: mod com relógio muda a cada segundo, e o leitor de tela leria sem parar. -->
  <section class="plugin-band" aria-label={m.plugin_band_label()}>
    {#if !isEmptyBand(tree)}
      <PluginNode node={tree} onPress={onPress ? (key) => onPress(BAND_SITE, key) : undefined} />
    {/if}
    {#if notice}
      <p class="notice" class:error={notice.error} role="status">
        {#if notice.href}
          <a href={notice.href} target="_blank" rel="noopener noreferrer">{notice.text}</a>
        {:else}
          {notice.text}
        {/if}
      </p>
    {/if}
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
  .notice { margin: var(--space-1) 0 0; font-family: var(--font-sans); color: var(--text-muted); }
  .notice.error { color: var(--error); }
  .notice a { color: var(--accent); }
</style>
