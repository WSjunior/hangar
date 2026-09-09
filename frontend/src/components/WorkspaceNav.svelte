<script lang="ts">
  import * as m from '../paraglide/messages';
  import type { WorkspaceView } from '../lib/workspaceCommands';

  interface Props {
    view: WorkspaceView;
    onSelect: (view: WorkspaceView) => void;
    onOpenCommand: () => void;
    // Trilho recolhido: so icones, empilhados. Quadro/Canvas forcam o recolhido, entao sem isto a
    // barra sumia junto com a sidebar e nao havia caminho de volta pro chat.
    rail?: boolean;
  }

  let { view, onSelect, onOpenCommand, rail = false }: Props = $props();

  // Rotulos CURTOS de proposito: a coluna tem 248px por padrao, e "Conversa" + "Quadro" + "Canvas"
  // + o botao de busca nao cabem sem cortar palavra no meio. O nome longo vive no title.
  // Orquestracao fica fora da barra: segue na paleta de comandos e na rota #/orq.
  const items: { id: WorkspaceView; label: string; title: string; icon: string }[] = [
    { id: 'chat', label: m.shell_chat_curto(), title: m.shell_conversa(),
      icon: 'M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z' },
    { id: 'board', label: m.shell_quadro(), title: m.shell_quadro(),
      icon: 'M3 4h5v16H3zM10 4h5v10h-5zM17 4h4v7h-4z' },
    { id: 'canvas', label: m.shell_canvas(), title: m.shell_canvas(),
      icon: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z' },
  ];
</script>

<div class="workspace-nav-wrap" class:rail>
  <nav class="workspace-nav" aria-label={m.shell_visualizacao()}>
    {#each items as item (item.id)}
      <button
        type="button"
        class:active={view === item.id}
        aria-current={view === item.id ? 'page' : undefined}
        title={item.title}
        aria-label={item.title}
        onclick={() => onSelect(item.id)}
      >{#if rail}<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="1.8" stroke-linejoin="round" aria-hidden="true"><path d={item.icon}/></svg>{:else}{item.label}{/if}</button>
    {/each}
  </nav>

  {#if !rail}
  <button
    type="button"
    class="command-button"
    onclick={onOpenCommand}
    aria-label={m.shell_abrir_busca()}
    title={m.shell_busca_comandos()}
  >
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
         stroke-width="2" stroke-linecap="round" aria-hidden="true">
      <circle cx="11" cy="11" r="7"></circle>
      <path d="m20 20-3.2-3.2"></path>
    </svg>
  </button>
  {/if}
</div>

<style>
  .workspace-nav-wrap {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
  }

  .workspace-nav {
    display: flex;
    flex: 1;
    min-width: 0;
    align-items: center;
    gap: 2px;
    padding: 3px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    /* `--surface-raised` em vez de uma alfa fixa de 94%: assim o segmentado entra no veu do papel de
       parede junto com o resto (CLAUDE.md, "Transparencia") e anda com o slider Solidez, em vez de
       ficar uma caixa chapada boiando sobre a foto. */
    background: var(--surface-raised);
  }

  .workspace-nav button {
    flex: 1;
    min-width: 0;      /* vence o min-width global de 44px: 3 botoes em 248px */
    height: 30px;
    min-height: 0;
    padding: 0 var(--space-1);
    overflow: hidden;
    border-radius: calc(var(--radius-lg) - 3px);
    color: var(--text-secondary);
    font-family: inherit;
    font-size: var(--text-xs);
    font-weight: 560;
    white-space: nowrap;
    text-overflow: ellipsis;
    transition: color 140ms var(--ease-out), background 140ms var(--ease-out);
  }

  .workspace-nav button:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .workspace-nav button.active {
    color: var(--text-primary);
    background: var(--bg-surface);
    box-shadow: inset 0 0 0 1px var(--border-default);
  }

  /* So o icone: o atalho fica no title, o texto "⌘K" custaria a largura de um dos tres botoes. */
  .command-button {
    flex-shrink: 0;
    width: 34px;
    min-width: 34px;
    height: 34px;
    min-height: 0;
    padding: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--bg-elevated) 94%, transparent);
    color: var(--text-secondary);
  }

  .command-button:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  /* Trilho: coluna de 3 botoes quadrados, mesma largura do botao de recolher do rail (40px). */
  .workspace-nav-wrap.rail { width: auto; justify-content: center; }
  .workspace-nav-wrap.rail .workspace-nav {
    flex: none; flex-direction: column; gap: 2px; padding: 3px; width: 40px;
  }
  .workspace-nav-wrap.rail .workspace-nav button {
    flex: none; width: 32px; height: 32px; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
  }

</style>
