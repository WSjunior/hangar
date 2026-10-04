<script lang="ts">
  import { useSessionServer } from '../lib/sessionServer';
  import { tick, untrack } from 'svelte';
  import BottomSheet from './BottomSheet.svelte';
  import Spinner from './Spinner.svelte';
  import { desktop } from '../lib/desktop.svelte';
  import { renderMarkdown } from '../lib/markdown';
  import * as m from '../paraglide/messages';
  import { fmtWhen, formataErro, historicoLateral, perguntaLateral } from '@hangar/core';
  import type { PerguntaLateral } from '@hangar/core';

  interface Props {
    open: boolean;
    sessionName: string;
    // Pergunta que veio do composer (`/btw …`): dispara sozinha ao abrir. Vazio = só abre o campo.
    pergunta?: string;
    onClose: () => void;
  }
  let { open, sessionName, pergunta = '', onClose }: Props = $props();
  const sessionServer = useSessionServer();

  let itens = $state<PerguntaLateral[]>([]);
  let emVoo = $state<string | null>(null);
  let erro = $state<string | null>(null);
  let texto = $state('');
  let listaEl = $state<HTMLDivElement | null>(null);
  let campoEl = $state<HTMLTextAreaElement | null>(null);

  // O BottomSheet mantém o componente montado entre aberturas: cada abertura recarrega o histórico
  // e uma resposta da abertura anterior não pode sobrescrever a nova. `untrack` porque `perguntar`
  // lê `emVoo`: rastreado, o efeito rodava de novo a cada resposta e disparava a MESMA pergunta
  // em loop (cada volta é uma chamada de API na sessão).
  let epoch = 0;
  $effect(() => {
    if (!open) return;
    const my = ++epoch;
    const q = pergunta;
    untrack(() => {
      erro = null;
      // Pergunta da abertura anterior ainda em voo: o "respondendo…" dela não é desta abertura, e
      // deixá-lo aceso travava a pergunta nova ("já tem uma em andamento") até a antiga voltar.
      emVoo = null;
      historicoLateral(sessionName, sessionServer())
        .then((h) => { if (my === epoch) { itens = h; void rolarFim(); } })
        .catch((e) => { if (my === epoch) erro = m.btw_historico_falhou({ erro: formataErro(e) ?? String(e) }); });
      if (q.trim()) void perguntar(q);
      else void tick().then(() => campoEl?.focus());
    });
  });

  async function rolarFim() {
    await tick();
    listaEl?.scrollTo({ top: listaEl.scrollHeight });
  }

  async function perguntar(q: string) {
    q = q.trim();
    if (!q) return;
    if (emVoo) {
      // O backend serializa por sessão; descartar calado deixava a 2ª pergunta sumir.
      erro = m.btw_em_andamento();
      texto = q;
      return;
    }
    // Mesma `epoch` do histórico, e pelo mesmo motivo: a folha fica montada entre aberturas e a
    // resposta pode demorar (sem terminal, até uns minutos). Sem esta guarda, a resposta de uma
    // pergunta feita ANTES de fechar caía na sessão aberta depois — entrava na lista dela e ainda
    // apagava o "respondendo…" de uma pergunta nova que estava em voo.
    const my = epoch;
    emVoo = q;
    erro = null;
    texto = '';
    void rolarFim();
    try {
      const r = await perguntaLateral(sessionName, q, sessionServer());
      if (my !== epoch) return;
      itens = [...itens, r];
    } catch (e) {
      if (my !== epoch) return;
      erro = m.btw_nao_deu({ erro: formataErro(e) ?? String(e) });
      texto = q;
    } finally {
      if (my === epoch) {
        emVoo = null;
        void rolarFim();
      }
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey && desktop.atual) {
      e.preventDefault();
      void perguntar(texto);
    }
  }
</script>

<BottomSheet {open} {onClose} ariaLabel={m.btw_titulo()} centered={desktop.atual} largura={760}>
  <div class="btw">
    <h2 class="title">{m.btw_titulo()}</h2>
    <p class="dica">{m.btw_dica()}</p>

    <div class="lista" bind:this={listaEl}>
      {#if itens.length === 0 && !emVoo}
        <p class="vazio">{m.btw_vazio()}</p>
      {/if}
      <!-- Trocas antigas nascem fechadas (só a pergunta e a hora): com todas abertas, uma resposta
           longa empurrava as anteriores e o campo pra fora da vista. `open` é valor INICIAL — depois
           de montado, quem manda é o clique do usuário, e o Svelte não reabre o que ele fechou. -->
      {#each itens as it, i (it.ts)}
        <details class="item" open={i === itens.length - 1}>
          <summary class="pergunta">
            <span class="seta" aria-hidden="true">›</span>
            <span class="q">{it.question}</span>
            <span class="hora">{fmtWhen(it.ts)}</span>
          </summary>
          <div class="resposta md">{@html renderMarkdown(it.answer)}</div>
          {#if it.fonte === 'pane'}<p class="aviso">{m.btw_talvez_cortada()}</p>{/if}
          {#if it.salvo === false}<p class="aviso">{m.btw_nao_guardada()}</p>{/if}
        </details>
      {/each}
      {#if emVoo}
        <div class="item">
          <p class="pergunta">{emVoo}</p>
          <Spinner label={m.btw_respondendo()} />
        </div>
      {/if}
    </div>

    {#if erro}
      <p class="erro">{erro}</p>
    {/if}

    <div class="campo">
      <textarea bind:this={campoEl} bind:value={texto} rows="2" placeholder={m.btw_placeholder()}
                aria-label={m.btw_titulo()} onkeydown={onKeydown} disabled={!!emVoo}></textarea>
      <button class="enviar" onclick={() => perguntar(texto)} disabled={!!emVoo || !texto.trim()}>
        {m.btw_perguntar()}
      </button>
    </div>
  </div>
</BottomSheet>

<style>
  .btw { padding: var(--space-4); display: flex; flex-direction: column; gap: var(--space-3); min-height: 56vh; max-height: 84vh; }
  .title { font-size: var(--text-base); font-weight: 600; color: var(--text-primary); }
  .dica { font-size: var(--text-xs); color: var(--text-muted); }

  .lista { position: relative; flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: var(--space-4); }
  .vazio { font-size: var(--text-sm); color: var(--text-muted); padding: var(--space-3) 0; }

  .item { display: flex; flex-direction: column; gap: var(--space-2); }
  .pergunta {
    font-size: var(--text-sm); color: var(--text-secondary);
    border-left: 3px solid var(--accent); padding-left: var(--space-3); word-break: break-word;
  }
  summary.pergunta {
    display: flex; align-items: baseline; gap: var(--space-3); cursor: pointer; list-style: none;
  }
  summary.pergunta::-webkit-details-marker { display: none; }
  summary.pergunta:hover { color: var(--text-primary); }
  .seta { flex: none; color: var(--text-muted); transition: transform 150ms var(--ease-out); }
  details[open] .seta { transform: rotate(90deg); }
  .q { flex: 1; min-width: 0; }
  /* Fechada, a pergunta é a linha do índice: uma linha só, pra a lista de trocas caber na vista. */
  details:not([open]) .q { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hora { flex: none; font-size: var(--text-xs); color: var(--text-muted); }
  .resposta { font-size: var(--text-sm); color: var(--text-primary); line-height: 1.5; word-break: break-word; }
  details .resposta { margin-top: var(--space-2); }
  .resposta :global(p) { margin: 0 0 var(--space-2); }
  .resposta :global(ul), .resposta :global(ol) { margin: 0 0 var(--space-2); padding-left: 1.2em; }
  .resposta :global(li) { margin: 2px 0; }
  .resposta :global(code) { padding: 0 4px; border-radius: 3px; background: var(--surface-raised); font-family: var(--font-mono); font-size: 12px; }
  .resposta :global(pre) { overflow-x: auto; padding: var(--space-2); border-radius: var(--radius-md); background: var(--surface-inset); }

  .erro { font-size: var(--text-sm); color: #e5484d; }
  .aviso { font-size: var(--text-xs); color: var(--text-muted); }

  .campo { display: flex; gap: var(--space-2); align-items: flex-end; }
  textarea {
    flex: 1; resize: none; font: inherit; font-size: var(--text-sm); color: var(--text-primary);
    background: var(--surface-inset); border: 1px solid var(--border-subtle); border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
  }
  .enviar {
    padding: var(--space-2) var(--space-3); border: none; border-radius: var(--radius-md);
    background: var(--accent); color: #fff; font-weight: 600; cursor: pointer;
  }
  .enviar:disabled { opacity: 0.5; cursor: default; }
</style>
