<script lang="ts">
  import { untrack } from 'svelte';
  import { serverIdentidade } from '../../lib/auth';
  import { createCodexAccountForServer, prepareCodexAccountForServer,
    getCodexPreparationForServer, startCodexAccountLoginForServer,
    getCodexAccountLoginForServer, cancelCodexAccountLoginForServer,
    codexAccountMessage, type Server, type CodexLoginAttempt, type CodexAccount } from '@hangar/core';
  import { copyText } from '../../lib/clipboard';
  import * as m from '../../paraglide/messages';

  let { server, accountId, oncomplete }: {
    server: Server; accountId?: string; oncomplete: () => void;
  } = $props();
  let name = $state('');
  let account = $state<string | undefined>();
  let attempt = $state<CodexLoginAttempt | null>(null);
  let sync = $state<CodexAccount['sync'] | null>(null);
  let busy = $state(false);
  let error = $state('');
  let generation = 0;
  let controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const identity = $derived(serverIdentidade(server));
  const url = $derived.by(() => {
    try {
      const u = new URL(attempt?.verification_url ?? '');
      return u.protocol === 'https:' && !u.username && !u.password ? u.href : null;
    } catch { return null; }
  });
  const failed = (e: unknown) => e instanceof Error ? e.message : m.codex_ui_login_error();

  function show(next: CodexLoginAttempt | null, g: number) {
    if (g !== generation) return;
    attempt = next;
    if (next?.status === 'completed') oncomplete();
  }

  async function read(s: Server, id: string, g: number, signal: AbortSignal) {
    try {
      const next = await getCodexAccountLoginForServer(s, id, signal);
      if (g !== generation) return;
      error = '';
      show(next, g);
      if (next?.status === 'waiting') timer = setTimeout(() => read(s, id, g, signal), 1000);
    } catch (e) {
      if (g === generation) error = failed(e);
    }
  }

  $effect(() => {
    void identity;
    const s = untrack(() => server), id = accountId;
    const g = ++generation;
    controller.abort();
    controller = new AbortController();
    clearTimeout(timer);
    account = id; attempt = null; sync = null; busy = false; error = ''; name = '';
    if (id) void read(s, id, g, controller.signal);
    return () => { ++generation; controller.abort(); clearTimeout(timer); };
  });

  async function start() {
    if (busy) return;
    const s = server, g = ++generation;
    const signal = controller.signal;
    clearTimeout(timer);
    busy = true; error = '';
    try {
      let id = account;
      if (!id) {
        const created = await createCodexAccountForServer(s, name.trim());
        if (g !== generation) return;
        account = id = created.id;
      }
      const prepared = await prepareCodexAccountForServer(s, id);
      if (g !== generation) return;
      sync = prepared;
      while (sync.status === 'running') {
        await new Promise<void>((resolve) => {
          const t = setTimeout(resolve, 1000);
          signal.addEventListener('abort', () => { clearTimeout(t); resolve(); }, { once: true });
        });
        if (g !== generation) return;
        const next = await getCodexPreparationForServer(s, id, signal);
        if (g !== generation) return;
        sync = next;
      }
      if (sync.status !== 'ready') return;
      const next = await startCodexAccountLoginForServer(s, id);
      if (g !== generation) return;
      show(next, g);
      if (next.status === 'waiting') timer = setTimeout(() => read(s, id, g, signal), 1000);
    } catch (e) {
      if (g === generation) error = failed(e);
    } finally { if (g === generation) busy = false; }
  }

  async function cancel() {
    if (!account || !attempt || busy) return;
    const s = server, id = account, attemptId = attempt.attempt_id, g = ++generation;
    clearTimeout(timer); busy = true; error = '';
    try { show(await cancelCodexAccountLoginForServer(s, id, attemptId), g); }
    catch (e) { if (g === generation) error = failed(e); }
    finally { if (g === generation) busy = false; }
  }
</script>

<div class="codex-login" aria-busy={busy}>
  <p>{m.codex_ui_oauth()}</p>
  {#if !account}
    <label>{m.novacred_nome_conta()}<input bind:value={name} disabled={busy} /></label>
  {/if}
  {#if sync?.status === 'running'}<p role="status">{m.codex_ui_preparing()}</p>
  {:else if sync?.status === 'ready'}<p role="status">{m.codex_ui_inherited()}</p>{/if}
  {#if sync?.trust_pending}<p role="status">{m.harness_codex_confianca()}</p>{/if}
  {#each sync?.issues ?? [] as issue, i (i)}<p role="alert">{codexAccountMessage(issue)}</p>{/each}
  {#if sync && sync.status !== 'running' && sync.status !== 'ready' && !sync.issues.length}
    <p role="alert">{m.codex_ui_prepare_error()}</p>
  {/if}
  {#if attempt?.status === 'waiting'}
    <p role="status">{m.novacred_codex_aguardando()}</p>
    {#if url}<a href={url} target="_blank" rel="noopener noreferrer">{url}</a>{/if}
    {#if attempt.user_code}
      <code>{attempt.user_code}</code>
      <button type="button" onclick={() => copyText(attempt!.user_code!).catch((e) => error = failed(e))}>{m.comum_copiar_codigo()}</button>
    {/if}
    <button type="button" disabled={busy} onclick={cancel}>{m.codex_ui_cancel_login()}</button>
    {#if error}<button type="button" onclick={() => account && read(server, account, generation, controller.signal)}>{m.novacred_codex_tentar()}</button>{/if}
  {:else if attempt?.status === 'completed'}
    <p role="status">{m.novacred_codex_concluido()}</p>
  {:else}
    {#if attempt?.error}<p role="alert">{codexAccountMessage(attempt.error)}</p>{/if}
    {#if attempt?.status === 'cancelled'}<p role="status">{m.codex_ui_cancelled()}</p>{/if}
    <button type="button" disabled={busy || (!account && !name.trim())} onclick={start}>
      {busy ? m.codex_ui_preparing() : m.contas_entrar()}
    </button>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
</div>

<style>
  .codex-login { display: flex; flex-direction: column; gap: var(--space-3); min-width: 0; }
  p { margin: 0; font-size: var(--text-sm); }
  label { display: flex; flex-direction: column; gap: var(--space-2); }
  input { background: var(--surface-inset); color: var(--text-primary); border: 1px solid var(--border-default); border-radius: var(--radius-sm); padding: var(--space-3); }
  button { background: var(--surface-raised); color: var(--text-primary); border: 1px solid var(--border-default); border-radius: var(--radius-sm); min-height: 44px; padding: var(--space-2); cursor: pointer; }
  button:disabled { opacity: .55; cursor: default; }
  a, code { overflow-wrap: anywhere; color: var(--accent); }
  [role='alert'] { color: var(--error); }
  button:focus-visible, input:focus-visible, a:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
</style>
