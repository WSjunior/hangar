<script lang="ts">
  import * as m from '../paraglide/messages';
  import { getSessionPlanPreview, type SessionPlanPreview } from '@hangar/core';
  import { renderMarkdown } from '../lib/markdown';
  import BottomSheet from './BottomSheet.svelte';

  interface Props {
    sessionName: string;
    provider: string;
    revision: string;
    desktop: boolean;
    codexPlan: string | null;
    disabled: boolean;
    onImplement: (plan: string) => Promise<void>;
  }
  let { sessionName, provider, revision, desktop, codexPlan, disabled, onImplement }: Props = $props();
  let metadata = $state<SessionPlanPreview | null>(null);
  let open = $state(false);
  let markdown = $state('');
  let loading = $state(false);
  let error = $state('');
  let sending = $state(false);
  let actionError = $state('');
  let discoveryError = $state('');
  let retries = $state(0);
  let dismissed = $state<string | null>(null);
  let generation = 0;
  let discoveredSession = '';
  let discoveryRetry = 0;
  const title = $derived(metadata?.name ?? m.chat_plan_proposto());
  const html = $derived(renderMarkdown(markdown, { joinWrapped: true }));

  $effect(() => {
    const name = sessionName;
    const source = provider;
    const state = revision;
    void retries;
    if (source !== 'claude') { metadata = null; discoveryError = ''; return; }
    if (state === 'working' && discoveredSession === name && discoveryRetry === retries) return;
    discoveredSession = name;
    discoveryRetry = retries;
    let active = true;
    getSessionPlanPreview(name, false).then((value) => {
      if (active) { metadata = value; discoveryError = ''; }
    }).catch(() => {
      if (active) discoveryError = m.chat_plan_erro();
    });
    return () => { active = false; };
  });

  $effect(() => {
    void sessionName;
    return () => { generation++; };
  });

  async function show() {
    const request = ++generation;
    open = true;
    error = '';
    markdown = '';
    loading = true;
    try {
      if (provider === 'codex') markdown = codexPlan ?? '';
      else {
        const result = await getSessionPlanPreview(sessionName);
        if (request !== generation) return;
        if (!result) error = m.chat_plan_ausente();
        else { metadata = result; markdown = result.markdown ?? ''; }
      }
    } catch (cause) {
      if (request !== generation) return;
      error = (cause as { status?: number }).status === 404 ? m.chat_plan_ausente() : m.chat_plan_erro();
    } finally {
      if (request === generation) loading = false;
    }
  }

  async function implement() {
    if (!codexPlan || disabled || sending) return;
    const plan = codexPlan;
    sending = true;
    actionError = '';
    try { await onImplement(plan); dismissed = plan; }
    catch (cause) { actionError = cause instanceof Error ? cause.message : m.chat_plan_erro(); }
    finally { sending = false; }
  }

  function close() { open = false; generation++; }
</script>

{#if metadata || codexPlan || actionError || discoveryError}
  <div class="plan-preview" role="group" aria-label={m.chat_plan_proposto()}>
    {#if metadata || codexPlan}
      <button class="plan-open" onclick={show}>
        <span>{m.chat_plan_ver()}</span>
        <span class="plan-name">{title}</span>
      </button>
    {/if}
    {#if codexPlan && dismissed !== codexPlan}
      <div class="plan-actions">
        <button class="primary-btn" onclick={implement} disabled={disabled || sending}>
          {sending ? m.chat_plan_iniciando() : m.chat_plan_implementar()}
        </button>
        <button class="ghost-btn" onclick={() => { dismissed = codexPlan; }} disabled={sending}>
          {m.chat_plan_continuar()}
        </button>
      </div>
    {/if}
    {#if actionError}<p class="error-msg" role="alert">{actionError}</p>{/if}
    {#if discoveryError}
      <p class="error-msg" role="alert">{discoveryError}</p>
      <button class="ghost-btn" onclick={() => { retries++; }}>{m.lista_tentar_novamente()}</button>
    {/if}
  </div>
{/if}

<BottomSheet {open} onClose={close} wide={desktop} centered={desktop}
  ariaLabel={m.chat_plan_ver()}>
  <header class="preview-header">
    <h2>{title}</h2>
    <button class="ghost-btn" onclick={close}>{m.sessao_fechar()}</button>
  </header>
  {#if metadata}<p class="plan-path">{metadata.path}</p>{/if}
  {#if loading}<p role="status">{m.chat_plan_carregando()}</p>
  {:else if error}
    <p class="error-msg" role="alert">{error}</p>
    <button class="ghost-btn" onclick={show}>{m.lista_tentar_novamente()}</button>
  {:else}<div class="prose">{@html html}</div>{/if}
</BottomSheet>

<style>
  .plan-preview { padding: var(--space-3) 0; display: grid; gap: var(--space-3); }
  .preview-header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); }
  .preview-header h2 { min-width: 0; overflow-wrap: anywhere; }
  .preview-header button { flex-shrink: 0; min-height: 44px; }
  .plan-open { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; min-height: 44px; width: fit-content; max-width: 100%; color: var(--accent); text-align: left; }
  .plan-name { color: var(--text-muted); font-size: var(--text-sm); overflow-wrap: anywhere; }
  .plan-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .plan-actions button { min-height: 44px; }
  .primary-btn { padding: var(--space-2) var(--space-4); background: var(--accent); color: var(--bg-base); border-radius: var(--radius-md); font-weight: 650; }
  .primary-btn:disabled { opacity: .5; cursor: default; }
  .ghost-btn { padding: var(--space-2) var(--space-3); color: var(--text-secondary); border-radius: var(--radius-md); }
  .ghost-btn:hover { background: var(--surface-raised); }
  .error-msg { color: var(--error); font-size: var(--text-sm); }
  .plan-path { color: var(--text-muted); font-size: var(--text-sm); overflow-wrap: anywhere; margin: var(--space-2) 0 var(--space-4); }
  .prose { overflow-wrap: anywhere; line-height: 1.65; color: var(--text-primary); }
  .prose :global(h1) { font-size: var(--text-xl); margin: var(--space-5) 0 var(--space-3); }
  .prose :global(h2) { font-size: var(--text-lg); margin: var(--space-5) 0 var(--space-2); }
  .prose :global(h3) { font-size: var(--text-base); margin: var(--space-4) 0 var(--space-2); }
  .prose :global(p), .prose :global(ul), .prose :global(ol) { margin: 0 0 var(--space-3); }
  .prose :global(ul), .prose :global(ol) { padding-left: 1.4em; }
  .prose :global(li) { margin: var(--space-1) 0; }
  .prose :global(code) { background: var(--surface-raised); font-family: var(--font-mono); font-size: .9em; padding: 0 .2em; border-radius: var(--radius-sm); }
  .prose :global(pre) { overflow-x: auto; background: var(--surface-inset); padding: var(--space-3); }
  .prose :global(pre code) { background: transparent; padding: 0; }
  .prose :global(a) { color: var(--accent); text-decoration: underline; }
  .prose :global(blockquote) { margin: var(--space-3); color: var(--text-secondary); }
</style>
