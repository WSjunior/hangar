<script lang="ts">
  import { PANE_CLOSE_KEY, type PluginPane } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    /** O painel desenhado: o da frente quando há vários. No celular sempre vira bloco acima do composer. */
    pane: PluginPane;
    /** Todos os painéis abertos; com mais de um, viram abas e só `pane` é desenhado, como no terminal. */
    tabs?: PluginPane[];
    onPress?: (site: string, key: string) => void;
    /** Troca de aba: quem chama decide se ela muda na hora ou espera o `shown_id` do servidor. */
    onShow?: (site: string) => void;
  }
  let { pane, tabs = [], onPress, onShow }: Props = $props();
  const tabbed = $derived(tabs.length > 1);
</script>

<section class="plugin-pane" aria-label={m.plugin_painel_label({ titulo: pane.title })}>
  <header>
    {#if tabbed}
      <div class="tabs" role="tablist" aria-label={m.plugin_abas_label()}>
        {#each tabs as tab (tab.id)}
          <button type="button" role="tab" class="tab" class:active={tab.id === pane.id}
                  aria-selected={tab.id === pane.id ? 'true' : 'false'}
                  onclick={() => { if (tab.id !== pane.id) onShow?.(tab.id); }}>{tab.title}</button>
        {/each}
      </div>
    {:else}
      <span class="title">{pane.title}</span>
    {/if}
    {#if onPress}
      <!-- Um ✕ só: fecha o painel da frente, como a marca do engine no terminal. -->
      <button type="button" class="close" aria-label={m.plugin_painel_fechar()}
              onclick={() => onPress(pane.id, PANE_CLOSE_KEY)}>✕</button>
    {/if}
  </header>
  <div class="body" role={tabbed ? 'tabpanel' : undefined}>
    <!-- Trocar de aba remonta o corpo: o hover aceso de um painel não passa para o outro. -->
    {#key pane.id}
      <PluginNode node={pane.tree} place={pane.columns} onPress={onPress ? (key) => onPress(pane.id, key) : undefined} />
    {/key}
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
  .tabs { display: flex; gap: 1ch; min-width: 0; overflow-x: auto; }
  .tab { font: inherit; color: var(--text-muted); background: transparent; border: 0; border-radius: var(--radius-sm); padding: 0 1ch; cursor: pointer; white-space: nowrap; min-height: 0; min-width: 0; }
  .tab.active { color: var(--text); font-weight: 600; background: var(--surface-raised); cursor: default; }
  .close { font: inherit; color: var(--text-muted); background: transparent; border: 0; cursor: pointer; padding: 0 0.5ch; min-height: 0; min-width: 0; }
  .body { position: relative; max-height: 40vh; overflow-y: auto; overflow-x: hidden; }
</style>
