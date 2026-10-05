<script lang="ts">
  import type { PluginInputKind } from '@hangar/core';
  import * as m from '../paraglide/messages';

  interface Props {
    label: string;
    placeholder: string;
    /** Valor que o mod desenhou. */
    value: string;
    /** O que o Enter faz; vazio, o rótulo do app. */
    submitLabel: string;
    /** Sem ele (sessão com terminal, ou servidor que não diz a fonte) o campo fica desabilitado, com a dica. */
    onInput?: (kind: PluginInputKind, value: string) => void;
  }
  let { label, placeholder, value, submitLabel, onInput }: Props = $props();
  let field: HTMLInputElement | undefined = $state();

  // O valor desenhado só entra com o campo fora de foco: um redesenho atrasado não apaga o que se digita.
  $effect(() => {
    const drawn = value;
    if (field && document.activeElement !== field && field.value !== drawn) field.value = drawn;
  });

  function submit() {
    if (field) onInput?.('submit', field.value);
  }
</script>

<span class="field">
  {#if label}<span class="label">{label}</span>{/if}
  <input bind:this={field} type="text" {placeholder} aria-label={label || placeholder} disabled={!onInput}
         oninput={(e) => onInput?.('change', e.currentTarget.value)}
         onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); submit(); } }} />
  {#if onInput}
    <button type="button" class="submit" onclick={submit}>{submitLabel || m.plugin_input_enviar()}</button>
  {:else}
    <span class="hint">{m.plugin_input_no_terminal()}</span>
  {/if}
</span>

<style>
  .field { display: inline-flex; align-items: center; flex-wrap: wrap; gap: 1ch; min-width: 0; }
  .label { white-space: nowrap; }
  input { font: inherit; color: inherit; background: var(--surface-raised); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 0 0.5ch; min-width: 12ch; flex: 1 1 16ch; }
  input:disabled { opacity: 0.6; }
  .submit { font: inherit; color: var(--accent); background: transparent; border: 0; padding: 0; cursor: pointer; min-height: 0; min-width: 0; }
  .hint { color: var(--text-muted); font-family: var(--font-sans); }
</style>
