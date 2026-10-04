<script lang="ts">
  import { untrack } from 'svelte';
  import { basename, deleteMergedWorktreesForServer, fetchWorktreesForServer, getWorktreesForServer,
           mergedWorktreeBatch, type WorktreeRepo, type WorktreeStatus } from '@hangar/core';
  import BottomSheet from '../components/BottomSheet.svelte';
  import Spinner from '../components/Spinner.svelte';
  import WorktreeSheet from '../components/WorktreeSheet.svelte';
  import { listOwnServers, onServersChanged, type Server } from '../lib/auth';
  import { worktreeStatus } from '../lib/worktreeStatus.svelte';
  import * as m from '../paraglide/messages';

  let { onBack }: { onBack?: () => void } = $props();
  let servidores = $state<Server[]>(listOwnServers());
  $effect(() => onServersChanged(() => { servidores = listOwnServers(); }));

  type Bloco = { servidor: Server; repos: WorktreeRepo[]; erro: string };
  let blocos = $state<Bloco[]>([]);
  let carregando = $state(true);
  let atualizando = $state(false);
  let aberta = $state<{ servidor: Server; path: string } | null>(null);
  let geracao = 0;

  async function carregar() {
    const meu = ++geracao;
    carregando = blocos.length === 0;
    const out = await Promise.all(servidores.map(async (s) => {
      try {
        const repos = await getWorktreesForServer(s);
        for (const r of repos) for (const w of r.worktrees) worktreeStatus.put(s.id, w);
        return { servidor: s, repos, erro: '' };
      } catch (e) { return { servidor: s, repos: [], erro: e instanceof Error ? e.message : String(e) }; }
    }));
    if (meu !== geracao) return;
    blocos = out;
    carregando = false;
  }

  // Primeiro mostra o que o disco já sabe; o fetch dos remotos (lento) vem depois e relê.
  async function atualizar() {
    atualizando = true;
    await Promise.allSettled(blocos.flatMap((b) => b.repos.map((r) => fetchWorktreesForServer(b.servidor, r.repo))));
    atualizando = false;
    await carregar();
  }

  // Por `serverId::repo`: o lote em andamento (botão desligado) e o aviso dele, que não é de leitura.
  let loteAndando = $state<string | null>(null);
  let erroLote = $state<Record<string, string>>({});
  const chaveLote = (b: Bloco, repo: string) => `${b.servidor.id}::${repo}`;

  // A confirmação congela o que a pessoa viu: o lote apaga essas e só essas.
  type Confirmacao = { bloco: Bloco; repo: string; deletable: WorktreeStatus[]; blocked: WorktreeStatus[] };
  let confirmando = $state<Confirmacao | null>(null);

  function pedirLote(b: Bloco, r: WorktreeRepo) {
    confirmando = { bloco: b, repo: r.repo, ...mergedWorktreeBatch(r) };
  }

  async function apagarMescladas() {
    if (!confirmando) return;
    const { bloco: b, repo, deletable } = confirmando;
    const k = chaveLote(b, repo);
    confirmando = null;
    loteAndando = k;
    delete erroLote[k];
    try {
      const removidas = await deleteMergedWorktreesForServer(b.servidor, repo, deletable);
      for (const p of removidas) worktreeStatus.drop(b.servidor.id, p);
      const ficaram = deletable.filter((w) => !removidas.includes(w.path));
      if (ficaram.length) erroLote[k] = m.worktree_lote_nao_apagou({ nomes: ficaram.map((w) => basename(w.path)).join(', ') });
    } catch (e) {
      erroLote[k] = e instanceof Error ? e.message : String(e);
    } finally {
      loteAndando = null;
    }
    await carregar();
  }

  // Mesma chave do Orq: lista recarregada com os mesmos servidores não refaz a leitura nem zera a tela.
  const chaveServidores = $derived(servidores.map((s) => `${s.id}|${s.baseUrl}|${s.token}`).join('\n'));
  $effect(() => { chaveServidores; untrack(() => { void carregar().then(atualizar); }); });

  const vazio = $derived(!carregando && blocos.every((b) => !b.erro && b.repos.length === 0));
</script>

<div class="wt">
  <header class="topo">
    {#if onBack}<button class="voltar" onclick={onBack} aria-label={m.orq_voltar()}>←</button>{/if}
    <h1>{m.worktrees_titulo()}</h1>
    {#if atualizando}<span class="muted">{m.worktrees_atualizando()}</span>{/if}
  </header>
  {#if carregando}
    <div class="centro"><Spinner /></div>
  {:else if vazio}
    <p class="vazio">{m.worktrees_vazio()}</p>
  {:else}
    {#each blocos as b (b.servidor.id)}
      {#if b.erro}<p class="erro" role="alert">{b.servidor.label}: {m.worktrees_erro({ motivo: b.erro })}</p>{/if}
      {#each b.repos as r (r.repo)}
        {@const lote = mergedWorktreeBatch(r)}
        <section class="repo">
          <h2>{basename(r.repo)}{servidores.length > 1 ? ` · ${b.servidor.label}` : ''}</h2>
          {#if lote.deletable.length}
            <button type="button" class="lote" disabled={loteAndando === chaveLote(b, r.repo)} onclick={() => pedirLote(b, r)}>
              {#if loteAndando === chaveLote(b, r.repo)}<Spinner />{/if}
              {m.worktree_apagar_mescladas({ n: lote.deletable.length })}
            </button>
          {/if}
          {#if erroLote[chaveLote(b, r.repo)]}<p class="erro" role="alert">{erroLote[chaveLote(b, r.repo)]}</p>{/if}
          <ul>
            {#each r.worktrees as w (w.path)}
              <li>
                <button type="button" class="linha" onclick={() => (aberta = { servidor: b.servidor, path: w.path })}>
                  <span class="nome">{basename(w.path)}</span>
                  <span class="muted">{w.branch} ← {w.base}</span>
                  <span>{w.merged ? m.worktree_juntada() : m.worktree_nao_juntada({ n: w.ahead })}</span>
                  {#if w.dirty}<span class="muted">{m.worktree_nao_commitados({ n: w.dirty })}</span>{/if}
                  {#if w.ignored.length}<span class="muted">{m.worktree_ignorados_perdem({ n: w.ignored.length })}</span>{/if}
                  {#if w.sessions.length}<span class="muted">{m.worktree_sessao_aberta({ nomes: w.sessions.join(', ') })}</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/each}
    {/each}
  {/if}
</div>
{#if aberta}
  <WorktreeSheet open={true} server={aberta.servidor} path={aberta.path} onClose={() => (aberta = null)} onDeleted={carregar} />
{/if}
{#if confirmando}
  <BottomSheet open={true} onClose={() => (confirmando = null)} ariaLabel={m.worktree_lote_titulo({ n: confirmando.deletable.length })}>
    <div class="sheet">
      <h2 class="title">{m.worktree_lote_titulo({ n: confirmando.deletable.length })}</h2>
      <ul class="itens">
        {#each confirmando.deletable as w (w.path)}
          <li>
            <span class="nome">{basename(w.path)}</span>
            {#if w.dirty || w.ignored.length}
              <span class="aviso">{m.worktree_apagar_perde()}</span>
              <ul class="perde">
                {#if w.dirty}<li>{m.worktree_nao_commitados({ n: w.dirty })}</li>{/if}
                {#each w.ignored as f (f)}<li>{f}</li>{/each}
              </ul>
            {:else}
              <span class="muted">{m.worktree_lote_nada_perde()}</span>
            {/if}
          </li>
        {/each}
      </ul>
      {#if confirmando.blocked.length}
        <p class="aviso">{m.worktree_lote_ficam()}</p>
        <ul class="itens">
          {#each confirmando.blocked as w (w.path)}
            <li>
              <span class="nome">{basename(w.path)}</span>
              <span class="muted">{w.sessions.length ? m.worktree_sessao_aberta({ nomes: w.sessions.join(', ') }) : m.worktree_lote_leitura_falhou()}</span>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="acoes">
        <button type="button" class="cancelar" onclick={() => (confirmando = null)}>{m.comum_cancelar()}</button>
        <button type="button" class="apagar" onclick={apagarMescladas}>{m.worktree_lote_confirmar()}</button>
      </div>
    </div>
  </BottomSheet>
{/if}

<style>
  .wt { display: flex; flex-direction: column; gap: 12px; height: 100%; overflow-y: auto; padding: 12px 16px 32px; }
  .topo { display: flex; align-items: center; gap: 8px; }
  .topo h1 { font-size: 1.15rem; margin: 0; flex: 1; }
  .voltar { background: none; border: 0; font-size: 1.2rem; min-width: 40px; min-height: 40px; color: inherit; }
  .repo h2 { font-size: 0.95rem; margin: 8px 0; }
  .repo ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  /* O botão global centraliza o conteúdo; a linha é uma ficha lida da esquerda. */
  .linha { width: 100%; text-align: left; display: flex; flex-direction: column; align-items: stretch; gap: 2px;
           padding: 10px 12px; border-radius: 10px; background: var(--surface-raised); border: 0; font: inherit; color: inherit; }
  .nome { font-weight: 600; }
  .muted { color: var(--text-muted); font-size: 0.85rem; }
  .erro { color: var(--error); }
  .vazio { color: var(--text-muted); text-align: center; padding: 32px 0; }
  .centro { display: flex; justify-content: center; padding: 32px; }
  .lote { align-self: flex-start; gap: 8px; margin-bottom: 8px; padding: 0 16px; border-radius: 8px;
          border: 1px solid var(--error); color: var(--error); font-weight: 600; }
  .lote:disabled { opacity: 0.6; }
  .sheet { padding: 16px; display: flex; flex-direction: column; gap: 10px; }
  .title { font-size: 1.05rem; margin: 0; }
  .itens { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .itens > li { display: flex; flex-direction: column; gap: 2px; }
  .aviso { margin: 0; color: var(--warning-text); font-size: 0.85rem; }
  .perde { margin: 0; padding-left: 18px; font-family: var(--font-mono); font-size: 0.8rem; overflow-wrap: anywhere; }
  .acoes { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  .cancelar { padding: 0 16px; border-radius: 8px; border: 1px solid var(--border-default); }
  .apagar { padding: 0 16px; border-radius: 8px; background: var(--error); color: #fff; font-weight: 600; }
</style>
