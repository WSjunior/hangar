<script lang="ts">
  import { useSessionServer } from '../../lib/sessionServer';
  // Painel de git do lado direito: uma aba por item aberto na coluna (arquivo da árvore de
  // trabalho ou commit). Commit mostra a lista de arquivos dele, e o diff do arquivo clicado abre
  // logo ABAIXO da linha — ler um commit é descer uma vez, não pular entre painel e lista.
  import { untrack } from 'svelte';
  import * as m from '../../paraglide/messages';
  import { getCommitFiles, type ChangedFile } from '@hangar/core';
  import { gitPainel, fecharAba, ativarAba, gitStoreDaSessao } from '../../lib/gitPainel.svelte';
  import FileIcon from '../files/FileIcon.svelte';
  import DiffView from './DiffView.svelte';

  interface Props { sessionName: string; }
  let { sessionName }: Props = $props();
  const sessionServer = useSessionServer();

  const git = $derived(gitStoreDaSessao(sessionName));
  const aba = $derived(gitPainel.abas.find((a) => a.id === gitPainel.ativa) ?? null);

  // Arquivos do commit da aba ativa. A guarda de `vez` evita que a resposta de um commit anterior
  // chegue por último e pinte a lista errada.
  let arquivos = $state<ChangedFile[]>([]);
  let falhou = $state(false);
  let abertoNoCommit = $state('');
  let vez = 0;
  $effect(() => {
    const a = aba;
    abertoNoCommit = '';
    if (!a || a.tipo !== 'commit') { arquivos = []; return; }
    const minha = ++vez;
    arquivos = [];
    falhou = false;
    getCommitFiles(sessionName, a.commit.hash, sessionServer())
      .then((r) => { if (minha === vez) arquivos = r.files; })
      .catch(() => { if (minha === vez) falhou = true; });
  });

  // Aba de arquivo: o diff é da árvore de trabalho e carrega ao ativar.
  // `untrack` é obrigatório: openFileDiff LÊ estado do store (busy, diffLoading) e, sem ele, esses
  // estados viram dependência do efeito — cada carga disparava outra, em laço infinito de /git/diff.
  $effect(() => {
    const a = aba;
    if (a?.tipo === 'arquivo') untrack(() => git.openFileDiff(a.path));
  });

  // Rolagem da faixa de abas: passando de ~4 abas a tira transborda, e sem seta ninguém descobre
  // que há aba escondida (a barra de rolagem horizontal não aparece até o ponteiro entrar nela).
  let faixaEl = $state<HTMLElement | null>(null);
  let temEsq = $state(false);
  let temDir = $state(false);
  function medirFaixa() {
    const e = faixaEl;
    if (!e) return;
    temEsq = e.scrollLeft > 1;
    temDir = e.scrollLeft + e.clientWidth < e.scrollWidth - 1;
  }
  function rolar(dir: 1 | -1) {
    faixaEl?.scrollBy({ left: dir * Math.max(120, (faixaEl.clientWidth || 0) * 0.7), behavior: 'smooth' });
  }
  $effect(() => {
    const e = faixaEl;
    if (!e) return;
    const ro = new ResizeObserver(medirFaixa);
    ro.observe(e);
    return () => ro.disconnect();
  });
  // Aba nova (ou a que sobrou depois de fechar outra) entra na vista sozinha: sem isto abrir o
  // quinto arquivo abria uma aba que ninguém vê.
  $effect(() => {
    void gitPainel.ativa; void gitPainel.abas.length;
    faixaEl?.querySelector<HTMLElement>('.aba.sel')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    medirFaixa();
  });

  function abrirDoCommit(path: string) {
    const a = aba;
    if (!a || a.tipo !== 'commit') return;
    if (abertoNoCommit === path) { abertoNoCommit = ''; return; }   // clicar de novo fecha
    abertoNoCommit = path;
    git.openCommitFileDiff(a.commit.hash, path);
  }
</script>

<div class="git-painel">
  <div class="faixa-linha">
  {#if temEsq}
    <button class="seta" onclick={() => rolar(-1)} aria-label={m.git_abas_anteriores()}>‹</button>
  {/if}
  <div class="faixa" role="tablist" bind:this={faixaEl} onscroll={medirFaixa}>
    {#each gitPainel.abas as a (a.id)}
      <div class="aba" class:sel={a.id === gitPainel.ativa}>
        <button class="aba-nome" role="tab" aria-selected={a.id === gitPainel.ativa}
                onclick={() => ativarAba(a.id)} title={a.rotulo}>
          {#if a.tipo === 'arquivo'}<FileIcon nome={a.path} />{/if}
          <span class="txt">{a.rotulo}</span>
        </button>
        <button class="aba-x" onclick={() => fecharAba(a.id)} aria-label={m.git_aba_fechar()}>×</button>
      </div>
    {/each}
  </div>
  {#if temDir}
    <button class="seta" onclick={() => rolar(1)} aria-label={m.git_abas_proximas()}>›</button>
  {/if}
  </div>

  {#if !aba}
    <p class="vazio">{m.git_painel_vazio()}</p>
  {:else if aba.tipo === 'arquivo'}
    <div class="corpo">
      <DiffView path={aba.path} rows={git.diffRows} loading={git.diffLoading}
                truncated={git.diffTruncated} compacto={true} />
    </div>
  {:else}
    <div class="corpo">
      <header class="cmt-topo">
        <span class="sha">{aba.commit.hash.slice(0, 8)}</span>
        <span class="autor">{aba.commit.author}</span>
        <span class="qtd">{arquivos.length === 1 ? m.git_arquivo_1() : m.git_arquivos({ n: arquivos.length })}</span>
      </header>
      {#if falhou}
        <p class="vazio">{m.git_arquivos_ler_erro()}</p>
      {:else}
        {#each arquivos as f (f.path)}
          <button class="arq" class:aberto={abertoNoCommit === f.path} onclick={() => abrirDoCommit(f.path)}>
            <span class="chev">{abertoNoCommit === f.path ? '▾' : '▸'}</span>
            <FileIcon nome={f.path} />
            <span class="cam">{f.path}</span>
          </button>
          {#if abertoNoCommit === f.path}
            <div class="inline">
              <DiffView path={f.path} rows={git.diffRows} loading={git.diffLoading}
                        truncated={git.diffTruncated} compacto={true} />
            </div>
          {/if}
        {/each}
      {/if}
    </div>
  {/if}
</div>

<style>
  .git-painel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  /* A costura de baixo passou pra linha inteira: as setas ficam ao lado da tira, não dentro dela
     (dentro elas rolariam junto com as abas). */
  .faixa-linha {
    display: flex; align-items: center; flex: none;
    border-bottom: 1px solid var(--border-subtle);
  }
  .faixa {
    display: flex; gap: 2px; align-items: center; overflow-x: auto; flex: 1; min-width: 0;
    padding: var(--space-1) var(--space-2);
    /* Barra de rolagem fora: ela só aparece com o ponteiro em cima (foi por isso que ninguém
       percebia a aba escondida) e ainda roubava altura da tira. Quem avisa agora são as setas. */
    scrollbar-width: none;
  }
  .faixa::-webkit-scrollbar { display: none; }
  .seta {
    flex: none; background: none; border: 0; cursor: pointer; padding: 2px 6px;
    color: var(--text-muted); font-size: var(--text-base); line-height: 1;
    transition: color 120ms cubic-bezier(0.2, 0, 0, 1);
  }
  .seta:hover { color: var(--text-primary); }
  /* Faixa com 4px de padding → aba em 8px (12 − 4): raio concêntrico com o card do painel. */
  .aba {
    display: flex; align-items: center; gap: var(--space-1);
    border-radius: 8px; padding-right: 2px; max-width: 160px; flex: none;
    transition: background-color 120ms cubic-bezier(0.2, 0, 0, 1);
  }
  .aba.sel { background: var(--fill-subtle); }
  .aba-nome {
    display: flex; align-items: center; gap: var(--space-1); min-width: 0;
    background: none; border: 0; color: var(--text-muted); font: inherit;
    font-size: var(--text-xs); padding: 4px var(--space-2); cursor: pointer;
  }
  .aba.sel .aba-nome { color: var(--text-primary); }
  .txt { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .aba-x {
    background: none; border: 0; color: var(--text-muted); cursor: pointer;
    font-size: var(--text-sm); line-height: 1; padding: 0 3px;
  }
  .aba-x:hover { color: var(--text-primary); }
  /* position: relative pela regra do overflow próprio: um .sr-only absoluto de dentro do diff
     escaparia pra área rolável da conversa. */
  .corpo { flex: 1; min-height: 0; overflow: auto; position: relative; padding: var(--space-2); }
  .cmt-topo {
    display: flex; align-items: center; gap: var(--space-2);
    font-size: var(--text-xs); color: var(--text-muted); padding: 0 var(--space-1) var(--space-2);
  }
  .sha { font-family: var(--font-mono); color: var(--accent); }
  .qtd { margin-left: auto; font-family: var(--font-mono); }
  /* `justify-content: flex-start`: o botão herdava `center` do estilo global e cada caminho
     começava num x diferente — a lista parecia escalonada em vez de alinhada. */
  .arq {
    display: flex; align-items: center; justify-content: flex-start;
    gap: var(--space-2); width: 100%;
    background: none; border: 0; color: var(--text-secondary); font: inherit;
    font-size: var(--text-xs); text-align: left; padding: 5px var(--space-2);
    border-radius: 8px; cursor: pointer;
    transition: background-color 120ms cubic-bezier(0.2, 0, 0, 1);
  }
  .arq:active { scale: 0.98; }
  .arq.aberto { background: var(--fill-subtle); color: var(--text-primary); }
  @media (hover: hover) and (pointer: fine) {
    .arq:hover { background: var(--fill-subtle); color: var(--text-primary); }
  }
  .chev { color: var(--text-muted); width: 10px; flex: none; }
  .cam { font-family: var(--font-mono); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .inline { padding: 0 0 var(--space-2) var(--space-4); }
  .vazio { padding: var(--space-4); color: var(--text-muted); font-size: var(--text-xs); }
</style>
