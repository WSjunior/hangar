<script lang="ts">
  import { untrack } from 'svelte';
  import { basename, deleteMergedWorktreesForServer, fetchWorktreesForServer, getWorktreesForServer,
           type WorktreeRepo } from '@hangar/core';
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

  // Por `serverId::repo`: o lote em andamento (botão desligado) e o erro dele, que não é de leitura.
  let loteAndando = $state<string | null>(null);
  let erroLote = $state<Record<string, string>>({});
  const chaveLote = (b: Bloco, repo: string) => `${b.servidor.id}::${repo}`;

  async function apagarJuntadas(b: Bloco, repo: string) {
    const k = chaveLote(b, repo);
    loteAndando = k;
    delete erroLote[k];
    try {
      await deleteMergedWorktreesForServer(b.servidor, repo);
    } catch (e) {
      erroLote[k] = e instanceof Error ? e.message : String(e);
      return;
    } finally {
      loteAndando = null;
    }
    await carregar();
  }

  // Mesma chave do Orq: lista recarregada com os mesmos servidores não refaz a leitura nem zera a tela.
  const chaveServidores = $derived(servidores.map((s) => `${s.id}|${s.baseUrl}|${s.token}`).join('\n'));
  $effect(() => { chaveServidores; untrack(() => { void carregar().then(atualizar); }); });

  const limpas = (r: WorktreeRepo) => r.worktrees.filter((w) => w.merged && !w.dirty && !w.ignored.length && !w.sessions.length);
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
        <section class="repo">
          <h2>{basename(r.repo)}{servidores.length > 1 ? ` · ${b.servidor.label}` : ''}</h2>
          {#if limpas(r).length}
            <button type="button" class="lote" disabled={loteAndando === chaveLote(b, r.repo)} onclick={() => apagarJuntadas(b, r.repo)}>
              {m.worktree_apagar_juntadas({ n: limpas(r).length })}
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

<style>
  .wt { display: flex; flex-direction: column; gap: 12px; height: 100%; overflow-y: auto; padding: 12px 16px 32px; }
  .topo { display: flex; align-items: center; gap: 8px; }
  .topo h1 { font-size: 1.15rem; margin: 0; flex: 1; }
  .voltar { background: none; border: 0; font-size: 1.2rem; min-width: 40px; min-height: 40px; color: inherit; }
  .repo h2 { font-size: 0.95rem; margin: 8px 0; }
  .repo ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .linha { width: 100%; text-align: left; display: flex; flex-direction: column; gap: 2px; padding: 10px 12px;
           border-radius: 10px; background: var(--surface-raised); border: 0; font: inherit; color: inherit; }
  .nome { font-weight: 600; }
  .muted { color: var(--text-muted); font-size: 0.85rem; }
  .erro { color: var(--error); }
  .vazio { color: var(--text-muted); text-align: center; padding: 32px 0; }
  .centro { display: flex; justify-content: center; padding: 32px; }
  .lote { align-self: flex-start; margin-bottom: 6px; }
</style>
