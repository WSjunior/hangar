<script lang="ts">
  import { untrack } from 'svelte';
  import { basename, deleteWorktreeForServer, getWorktreeForServer, type Server, type WorktreeStatus } from '@hangar/core';
  import BottomSheet from './BottomSheet.svelte';
  import Spinner from './Spinner.svelte';
  import { worktreeStatus } from '../lib/worktreeStatus.svelte';
  import * as m from '../paraglide/messages';

  interface Props { open: boolean; server: Server; path: string; onClose: () => void; onDeleted?: () => void }
  let { open, server, path, onClose, onDeleted }: Props = $props();

  let st = $state<WorktreeStatus | null>(null);
  let erro = $state('');
  let apagando = $state(false);
  let apagarBranch = $state(false);
  let geracao = 0;

  // Só abrir e trocar de caminho releem: lista de servidores recarregada traz objeto novo do mesmo servidor.
  $effect(() => {
    if (!open) return;
    const p = path;
    const meu = ++geracao;
    st = null; erro = ''; apagarBranch = false;
    const s = untrack(() => server);
    getWorktreeForServer(s, p)
      .then((r) => { if (meu !== geracao) return; st = r; worktreeStatus.put(s.id, r); })
      .catch((e) => { if (meu === geracao) erro = e instanceof Error ? e.message : String(e); });
  });

  const perde = $derived(st ? [...(st.dirty ? [m.worktree_nao_commitados({ n: st.dirty })] : []), ...st.ignored] : []);

  async function apagar() {
    if (!st) return;
    apagando = true; erro = '';
    try {
      await deleteWorktreeForServer(server, { repo: st.repo, path: st.path, confirm: perde.length > 0, delete_branch: apagarBranch });
      worktreeStatus.drop(server.id, st.path);
      onDeleted?.();
      onClose();
    } catch (e) {
      erro = e instanceof Error ? e.message : String(e);
    } finally {
      apagando = false;
    }
  }
</script>

<BottomSheet {open} {onClose} ariaLabel={m.worktrees_titulo()}>
  <div class="sheet">
    <h2 class="title">{basename(path)}</h2>
    {#if !st && !erro}
      <div class="centro"><Spinner /></div>
    {:else if erro && !st}
      <p class="erro" role="alert">{m.worktrees_erro({ motivo: erro })}</p>
    {:else if st}
      <p class="linha">{st.branch} ← {st.base}</p>
      <p class="linha">{st.merged ? m.worktree_juntada() : m.worktree_nao_juntada({ n: st.ahead })}</p>
      {#if st.sessions.length}<p class="linha">{m.worktree_sessao_aberta({ nomes: st.sessions.join(', ') })}</p>{/if}
      {#if st.closed}<p class="linha muted">{m.worktree_conversas_fechadas({ n: st.closed })}</p>{/if}
      {#if st.sessions.length}
        <p class="aviso">{m.worktree_bloqueada({ nomes: st.sessions.join(', ') })}</p>
      {:else}
        {#if perde.length}
          <p class="aviso">{m.worktree_apagar_perde()}</p>
          <ul class="perde">{#each perde as p (p)}<li>{p}</li>{/each}</ul>
        {/if}
        <p class="linha muted">{m.worktree_apagar_conversas({ branch: st.main_branch ?? st.base ?? '' })}</p>
        {#if st.merged}
          <p class="linha muted">{m.worktree_apagar_branch_juntada({ branch: st.branch ?? '', base: st.base ?? '' })}</p>
        {:else}
          <p class="linha muted">{m.worktree_apagar_branch_fica({ branch: st.branch ?? '' })}</p>
          <label class="linha"><input type="checkbox" bind:checked={apagarBranch} /> {m.worktree_apagar_branch_tambem({ n: st.ahead })}</label>
        {/if}
        {#if erro}<p class="erro" role="alert">{erro}</p>{/if}
        <button type="button" class="apagar" disabled={apagando} onclick={apagar}>{m.worktree_apagar()}</button>
      {/if}
    {/if}
  </div>
</BottomSheet>

<style>
  .sheet { padding: 16px; display: flex; flex-direction: column; gap: 8px; }
  .title { font-size: 1.05rem; margin: 0; }
  .linha { margin: 0; }
  .muted { color: var(--text-muted); }
  .aviso { margin: 0; color: var(--warning-text); }
  .erro { margin: 0; color: var(--error); }
  .perde { margin: 0; padding-left: 18px; font-family: var(--font-mono); font-size: 0.85rem; }
  .centro { display: flex; justify-content: center; padding: 16px; }
  .apagar { align-self: flex-end; min-height: 40px; padding: 0 16px; border-radius: 8px;
            background: var(--error); color: #fff; border: 0; }
</style>
