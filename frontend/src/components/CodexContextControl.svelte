<script lang="ts">
  import { codexOpcoes, type Server } from '@hangar/core';
  import * as m from '../paraglide/messages';

  let { server, busy = $bindable(false) }: { server: Server | null; busy?: boolean } = $props();
  let enabled = $state(false);
  let ready = $state(false);
  let error = $state('');
  let context: { server: Server | null; controller: AbortController };

  async function update(ctx: typeof context, value?: boolean) {
    busy = true;
    error = '';
    try {
      const result = await codexOpcoes(ctx.server, ctx.controller.signal, value);
      if (context !== ctx || ctx.controller.signal.aborted) return;
      enabled = result.contexto_estendido;
      ready = true;
    } catch (e) {
      if (context === ctx && !ctx.controller.signal.aborted) {
        error = e instanceof Error ? e.message : m.comum_falha_aplicar();
      }
    } finally {
      if (context === ctx && !ctx.controller.signal.aborted) busy = false;
    }
  }

  $effect(() => {
    const ctx = { server, controller: new AbortController() };
    context = ctx;
    ready = false;
    void update(ctx);
    return () => { ctx.controller.abort(); busy = false; };
  });
</script>

<div class="context-control">
  <label>
    <span>{m.codex_contexto_titulo()}</span>
    <input type="checkbox" role="switch" checked={enabled} disabled={busy || !ready}
      onchange={(e) => { e.currentTarget.checked = enabled; void update(context, !enabled); }} />
  </label>
  <p>{m.codex_contexto_padrao()}</p>
  {#if busy}<p role="status">{m.comum_carregando()}</p>{/if}
  {#if error}
    <p role="alert">{error}</p>
    <button type="button" disabled={busy} onclick={() => update(context)}>{m.config_server_tentar_de_novo()}</button>
  {/if}
</div>

<style>
  .context-control { display: grid; grid-column: 1 / -1; gap: var(--space-2); }
  label { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); min-height: 44px; font-size: var(--text-sm); }
  input { width: 20px; height: 20px; accent-color: var(--accent); }
  p { margin: 0; font-size: var(--text-xs); color: var(--text-secondary); }
  [role="alert"] { color: var(--error); }
  button { min-height: 44px; background: var(--surface-raised); border-radius: var(--radius-md); }
</style>
