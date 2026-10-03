<script lang="ts">
  import { untrack } from 'svelte';
  import { getFolderBranchesForServer, type FolderBranches, type Server, type WorktreeChoice } from '@hangar/core';
  import * as m from '../paraglide/messages';

  interface Props { server: Server; cwd: string; value: WorktreeChoice | null; sessionName: string;
                    onChange: (v: WorktreeChoice | null) => void }
  let { server, cwd, value, sessionName, onChange }: Props = $props();

  let info = $state<FolderBranches | null>(null);
  let mode = $state<'current' | 'existing' | 'new'>('current');
  // undefined, não '': o select ligado adota a primeira opção, e a tela mostra o que vai ser enviado.
  let existing = $state<string | undefined>(undefined);
  let base = $state('');
  let branchName = $state('');

  const serverId = $derived(server.id);
  $effect(() => {
    const alvo = cwd, id = serverId;
    // A lista de servidores é refeita com objetos novos a cada recarga: só a identidade reabre a leitura,
    // senão a recarga apagava o que a pessoa digitou.
    const s = untrack(() => server);
    // Pasta nova: escolha da anterior não vale aqui (a branch pode nem existir neste repositório).
    info = null; mode = 'current'; existing = undefined; branchName = '';
    const atual = () => alvo === cwd && id === serverId;
    getFolderBranchesForServer(s, alvo).then((r) => { if (atual()) { info = r; base = r.current ?? ''; } })
      .catch(() => { if (atual()) info = null; });   // pasta sem git: o seletor some
  });
  const others = $derived(info ? [...info.branches, ...info.remotes].filter((b) => b !== info?.current) : []);

  $effect(() => {
    if (!info) onChange(null);
    else if (mode === 'existing' && existing) onChange({ branch: existing });
    else if (mode === 'new') onChange({ branch: (branchName || sessionName).trim(), new_branch: true, base: base || null });
    else onChange(null);
  });
</script>

{#if info}
  <div class="field">
    <span class="field-label">{m.native_create_checkout_branch()}</span>
    <select class="field-input" bind:value={mode}>
      <option value="current">{m.native_create_checkout_current()}{info.current ? ` · ${info.current}` : ''}</option>
      {#if others.length}<option value="existing">{m.native_create_checkout_worktree()}</option>{/if}
      <option value="new">{m.worktree_nova_branch({ base: base || info.current || '' })}</option>
    </select>
    {#if mode === 'existing'}
      <select class="field-input" bind:value={existing}>
        {#each others as b (b)}<option value={b}>{b}</option>{/each}
      </select>
    {:else if mode === 'new'}
      <input class="field-input" type="text" placeholder={sessionName} bind:value={branchName} aria-label={m.worktree_nome_branch()} />
      <select class="field-input" bind:value={base} aria-label={m.worktree_base()}>
        {#each [info.current, ...others].filter(Boolean) as b (b)}<option value={b}>{b}</option>{/each}
      </select>
    {/if}
    {#if mode !== 'current'}<p class="hint">{m.worktree_modo()} — {m.worktree_modo_ajuda()}</p>{/if}
  </div>
{/if}

<style>
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-secondary); }
</style>
