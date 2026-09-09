<script lang="ts">
  import * as m from '../paraglide/messages';
  import Popover from './Popover.svelte';
  import { rotuloPermissao } from '../lib/permissaoRotulo';

  const CLAUDE_MODES = ['plan', 'auto', 'manual', 'acceptEdits', 'bypassPermissions', 'dontAsk'];

  interface Props {
    provider: 'claude' | 'codex';
    current: string | null;
    modes?: string[];
    loading?: boolean;
    error?: string | null;
    onOpen?: () => void;
    onApply: (mode: string) => Promise<void> | void;
  }

  let { provider, current, modes = [], loading = false, error = null, onOpen, onApply }: Props = $props();
  let open = $state(false);
  let anchor = $state<HTMLElement | null>(null);
  let applying = $state<string | null>(null);

  const options = $derived(provider === 'codex' ? ['default', 'plan'] : CLAUDE_MODES);
  const available = $derived(new Set(provider === 'codex' ? options : modes));
  const label = $derived(!current ? m.chat_mode_label() : current === 'plan'
    ? m.chat_mode_plan()
    : provider === 'codex'
      ? m.chat_mode_normal()
      : current ? rotuloPermissao(current) : m.chat_mode_label());

  function optionLabel(mode: string): string {
    if (mode === 'default') return m.chat_mode_normal();
    if (mode === 'plan') return m.chat_mode_plan();
    return rotuloPermissao(mode);
  }

  function show() {
    open = true;
    onOpen?.();
  }

  async function choose(mode: string) {
    if (applying || !available.has(mode)) return;
    if (mode === current) { open = false; return; }
    applying = mode;
    try {
      await onApply(mode);
      open = false;
    } catch {
      // O chamador mantém a mensagem junto do controle e o menu aberto permite tentar de novo.
    } finally {
      applying = null;
    }
  }
</script>

<div class="mode-control">
  <button
    type="button"
    class="trigger"
    class:planning={current === 'plan'}
    bind:this={anchor}
    onclick={show}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-pressed={current === 'plan'}
    aria-label={`${m.chat_mode_label()}: ${label}`}
    title={provider === 'claude' ? m.permissao_atalho() : m.codex_modo_atalho()}
    data-mode-trigger
  >
    <span>{label}</span>
    <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true">
      <path d="m4 6 4 4 4-4" />
    </svg>
  </button>
  {#if error}<span class="error" role="alert">{error}</span>{/if}
</div>

<Popover {open} {anchor} onClose={() => (open = false)} width={270} ariaLabel={m.chat_mode_label()}>
  {#if loading}
    <p class="status">{m.comum_carregando()}</p>
  {:else}
    <ul class="options">
      {#each options as mode (mode)}
        {@const enabled = available.has(mode)}
        <li>
          <button
            type="button"
            class="option"
            class:selected={current === mode}
            disabled={!enabled || applying !== null}
            onclick={() => choose(mode)}
            aria-pressed={current === mode}
            title={!enabled ? m.chat_mode_only_creation() : undefined}
            data-mode-option
            data-mode={mode}
          >
            <span>{optionLabel(mode)}</span>
            {#if !enabled}<span class="unavailable">{m.chat_mode_unavailable()}</span>{/if}
            {#if applying === mode}<span aria-hidden="true">…</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</Popover>

<style>
  .mode-control { display: flex; align-items: center; gap: var(--space-2); min-width: 0; }
  .trigger {
    display: inline-flex; align-items: center; gap: 6px; height: 30px; min-height: 0; min-width: 0; max-width: 100%;
    padding: 4px var(--space-2); border-radius: var(--radius-md); background: var(--surface-raised);
    color: var(--text-secondary); font: inherit; font-size: var(--text-xs); cursor: pointer;
  }
  .trigger > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .trigger > svg { flex: none; }
  .trigger.planning { background: var(--accent-dim); color: var(--accent); font-weight: 650; }
  .error { color: var(--error); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .options { list-style: none; margin: 0; padding: 4px 0; }
  .option {
    display: flex; align-items: center; gap: var(--space-2); width: 100%; padding: 8px 10px;
    border: 0; background: transparent; color: var(--text-primary); font: inherit;
    font-size: var(--text-sm); text-align: start; cursor: pointer;
  }
  .option > :first-child { flex: 1; min-width: 0; }
  .option:hover:not(:disabled) { background: var(--bg-hover); }
  .option.selected { background: var(--accent-dim); }
  .option:disabled { cursor: not-allowed; color: var(--text-muted); }
  .unavailable { font-size: var(--text-xs); color: var(--text-muted); }
  .status { margin: 0; padding: 14px 10px; color: var(--text-muted); font-size: var(--text-sm); text-align: center; }
</style>
