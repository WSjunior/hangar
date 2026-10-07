<script lang="ts" module>
  /** Resultado do último clique num botão de mod: "Copiado", o erro, o link que o navegador
   *  bloqueou ou um toque que copia. */
  export type PluginNotice = { text: string; error: boolean; href?: string; action?: () => void };
</script>

<script lang="ts">
  import { BAND_SITE, isEmptyBand, type PluginControl, type PluginInputKind, type PluginNode as Node } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import PluginNode from './PluginNode.svelte';

  interface Props {
    /** Árvore do `AbovePrompt` como o Claude Code a desenhou, de todos os mods juntos. */
    tree: Node;
    /** Largura, em colunas, para a qual a faixa foi desenhada (`columns` do evento); null num servidor antigo. */
    columns?: number | null;
    /** Clique num botão de mod; sem ele, os botões são só rótulo. */
    onPress?: (site: string, button: PluginControl) => void;
    /** Digitação num `Input` da faixa; só sem terminal. */
    onInput?: (site: string, field: PluginControl, kind: PluginInputKind, value: string) => void;
    /** Resultado do último clique; some sozinho. */
    notice?: PluginNotice | null;
  }
  let { tree, columns = null, onPress, onInput, notice = null }: Props = $props();
</script>

{#if !isEmptyBand(tree) || notice}
  <!-- Sem aria-live na faixa: mod com relógio muda a cada segundo, e o leitor de tela leria sem parar. -->
  <section class="plugin-band" aria-label={m.plugin_band_label()}>
    {#if !isEmptyBand(tree)}
      <PluginNode node={tree} place={columns} onPress={onPress ? (button) => onPress(BAND_SITE, button) : undefined}
                  onInput={onInput ? (field, kind, value) => onInput(BAND_SITE, field, kind, value) : undefined} />
    {/if}
    {#if notice}
      <p class="notice" class:error={notice.error} role="status">
        {#if notice.href}
          <a href={notice.href} target="_blank" rel="noopener noreferrer">{notice.text}</a>
        {:else if notice.action}
          <button type="button" class="notice-action" onclick={notice.action}>{notice.text}</button>
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
    color: var(--text-primary);
    /* Fundo do cartão `absolute` sem cor própria: a mesma cor da faixa, opaca (com papel de parede o
       `--surface-inset` fica translúcido, e a linha de baixo atravessaria o cartão). */
    --plugin-place-bg: var(--bg-base);
    background: var(--surface-inset);
    border-radius: var(--radius-md);
    overflow-x: auto;
    overflow-y: hidden;
  }
  .notice { margin: var(--space-1) 0 0; font-family: var(--font-ui); color: var(--text-muted); }
  .notice.error { color: var(--error); }
  .notice a { color: var(--accent); }
  .notice-action { font: inherit; color: var(--accent); background: transparent; border: 0; padding: 0; cursor: pointer; text-decoration: underline; }
</style>
