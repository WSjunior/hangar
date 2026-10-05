<script lang="ts">
  import { PANE_CLOSE_KEY, type PluginPane } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    /** Painel que um mod abriu; no celular sempre vira bloco acima do composer. */
    pane: PluginPane;
    onPress?: (site: string, key: string) => void;
  }
  let { pane, onPress }: Props = $props();
</script>

<section class="plugin-pane" aria-label={m.plugin_painel_label({ titulo: pane.title })}>
  <header>
    <span class="title">{pane.title}</span>
    {#if onPress}
      <button type="button" class="close" aria-label={m.plugin_painel_fechar()}
              onclick={() => onPress(pane.id, PANE_CLOSE_KEY)}>✕</button>
    {/if}
  </header>
  <div class="body">
    <PluginNode node={pane.tree} place={pane.columns} onPress={onPress ? (key) => onPress(pane.id, key) : undefined} />
  </div>
</section>

<style>
  .plugin-pane {
    position: relative;
    margin: 0 var(--space-3) var(--space-1);
    padding: var(--space-1) var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--text);
    background: var(--surface-inset);
    border-radius: var(--radius-md);
  }
  header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-2); }
  .title { font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .close { font: inherit; color: var(--text-muted); background: transparent; border: 0; cursor: pointer; padding: 0 0.5ch; min-height: 0; min-width: 0; }
  .body { position: relative; max-height: 40vh; overflow-y: auto; overflow-x: hidden; }
</style>
