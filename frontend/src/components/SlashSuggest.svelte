<script lang="ts">
  import { slashMatches, type CommandInfo } from '@hangar/core';
  import * as m from '../paraglide/messages';

  // Tira inline de autocomplete acima do textarea. Aparece enquanto a palavra sob o cursor e um
  // `/nome` sem argumento (`query` = o que veio depois da barra; null = nada sendo digitado).
  // Renderiza no fluxo normal, acima do input, pra nunca ficar atras do teclado.
  interface Props {
    commands: CommandInfo[];
    query: string | null;
    onPick: (cmd: CommandInfo) => void;
    onComplete: (cmd: CommandInfo) => void;
    onDismiss: () => void;
    listboxId: string;
    activeOptionId?: string;
  }
  let {
    commands, query, onPick, onComplete, onDismiss, listboxId,
    activeOptionId = $bindable(),
  }: Props = $props();

  let selectedName = $state('');
  let rows: HTMLButtonElement[] = [];

  const matches = $derived(slashMatches(commands, query));
  const selected = $derived(matches.find((c) => c.name === selectedName) ?? matches[0]);
  const selectedIndex = $derived(selected ? matches.indexOf(selected) : -1);

  $effect(() => {
    activeOptionId = selectedIndex >= 0 ? `${listboxId}-${selectedIndex}` : undefined;
  });

  export function handleKeydown(event: KeyboardEvent): boolean {
    if (!selected || event.ctrlKey || event.altKey || event.metaKey) return false;
    // Esc fecha a lista e deixa o texto como esta; ela volta quando a palavra mudar.
    if (event.key === 'Escape') {
      event.preventDefault();
      onDismiss();
      return true;
    }
    if (event.key === 'Tab' && !event.shiftKey) {
      event.preventDefault();
      onComplete(selected);
      return true;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return false;

    event.preventDefault();
    const current = Math.max(0, matches.indexOf(selected));
    const next = (current + (event.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length;
    selectedName = matches[next].name;
    rows[next]?.scrollIntoView?.({ block: 'nearest' });
    return true;
  }

  function badge(source: CommandInfo['source']): string {
    return source === 'builtin' ? 'base' : source === 'plugin' ? 'plugin' : 'skill';
  }
</script>

{#if matches.length > 0}
  <div class="suggest" id={listboxId} role="listbox" aria-label={m.slash_sugestoes()}>
    {#each matches as c, index (c.name)}
      <button bind:this={rows[index]} class="row" class:selected={c === selected} role="option"
        id="{listboxId}-{index}" tabindex="-1" aria-selected={c === selected}
        onmouseenter={() => (selectedName = c.name)} onclick={() => onPick(c)}>
        <span class="name">{c.display}</span>
        {#if c.description}<span class="desc">{c.description}</span>{/if}
        <span class="badge badge--{c.source}">{badge(c.source)}</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .suggest {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 168px; /* ~3-4 linhas; rola se passar */
    overflow-y: auto;
    margin-bottom: var(--space-1);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--bg-surface);
    padding: var(--space-1);
  }

  .row {
    width: 100%;
    min-height: 44px;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    text-align: left;
    background: transparent;
    transition: background 160ms var(--ease-out);
  }

  .row:active {
    background: var(--bg-hover);
  }

  .row.selected {
    background: var(--bg-hover);
  }

  .name {
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-primary);
    flex-shrink: 0;
  }

  .desc {
    font-size: var(--text-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }

  .badge {
    flex-shrink: 0;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: var(--radius-full);
    color: var(--text-secondary);
    background: var(--bg-hover);
  }

  .badge--skill {
    color: var(--accent);
    background: var(--accent-dim);
  }

  .badge--plugin {
    color: var(--warning);
    background: rgba(255, 159, 10, 0.14);
  }
</style>
