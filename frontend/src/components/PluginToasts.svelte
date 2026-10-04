<script lang="ts">
  import type { PluginToast } from '@hangar/core';
  import * as m from '../paraglide/messages';

  interface Props {
    /** Avisos que os mods mostraram no terminal e ainda não venceram, do mais antigo ao mais novo. */
    toasts: PluginToast[];
    /** Toque no aviso: tira da tela antes do prazo, como o clique no terminal. */
    onDismiss: (id: string) => void;
  }
  let { toasts, onDismiss }: Props = $props();
</script>

<!-- O contêiner fica montado vazio: o leitor de tela só anuncia o que entra numa região que já existia. -->
<div class="plugin-toasts" role="status">
  {#each toasts as t (t.id)}
    <button type="button" class="toast" title={m.chat_problema_dispensar()} onclick={() => onDismiss(t.id)}>
      {#if t.plugin}<span class="mod">{t.plugin}</span>{/if}
      <span class="text">{t.text}</span>
    </button>
  {/each}
</div>

<style>
  /* Canto superior direito da conversa, como no terminal. Acima da navbar (20): o degradê dela
     desce além de --nav-h e apagaria o nome do mod. */
  .plugin-toasts {
    position: absolute;
    top: calc(var(--nav-h, 0px) + var(--space-2));
    right: var(--space-3);
    z-index: 21;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-2);
    max-width: min(22rem, calc(100% - 2 * var(--space-3)));
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: var(--space-2) var(--space-3);
    font: inherit;
    font-size: var(--text-sm);
    text-align: left;
    color: var(--text-primary);
    /* Opaco, como as pílulas que flutuam sobre a conversa: o texto de baixo vazaria por --surface-raised. */
    background: var(--bg-elevated, var(--bg-base));
    border: 1px solid var(--border-default);
    border-radius: var(--radius-md);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .mod { font-size: var(--text-xs); color: var(--text-muted); }
  .text { white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
