<script lang="ts">
  // Compartilhar UMA sessão: gera o link de uso único e mostra quem tem acesso. O link só existe
  // na hora em que nasce (o backend guarda o hash), então ele vive no estado desta folha.
  import * as m from '../paraglide/messages';
  import BottomSheet from './BottomSheet.svelte';
  import { desktop } from '../lib/desktop.svelte';
  import { sessionServerFor } from '../lib/sessionServer';
  import { copyText } from '../lib/clipboard';
  import {
    createShare, listShares, revokeShare, revokeAllShares, relativeTime, resetsIn, sharePrereqs,
    SharePrerequisiteError, type ShareInfo, type ShareCreated, type SharePrereqs,
  } from '@hangar/core';

  interface Props { open: boolean; name: string; serverId: string; onClose: () => void }
  let { open, name, serverId, onClose }: Props = $props();
  // Pelo servidor recebido, sem trocar o ativo do app enquanto a folha espera a resposta.
  const server = () => sessionServerFor(serverId)();

  const CONFERIR_MS = 3000;
  const CONFERIR_ATE_MS = 5 * 60_000;

  let shares = $state<ShareInfo[] | null>(null);
  let erroLista = $state('');
  let criado = $state<ShareCreated | null>(null);
  let gerando = $state(false);
  let erroGerar = $state('');
  let preRequisito = $state<SharePrereqs | null>(null);
  let copiado = $state(false);
  let comandoCopiado = $state(false);
  let conferindo = $state(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));

  async function carregar() {
    erroLista = '';
    shares = null;
    try {
      shares = (await listShares(name, server())).shares;
    } catch (e) {
      erroLista = msg(e);
    }
  }

  $effect(() => {
    if (!open) return;
    criado = null;
    erroGerar = '';
    preRequisito = null;
    copiado = false;
    void carregar();
    // Fechar a folha (ou desmontar) para a conferência: ninguém está esperando a resposta.
    return pararConferencia;
  });

  function pararConferencia() {
    clearInterval(timer);
    timer = undefined;
    conferindo = false;
  }

  // Liberar acontece no site do Tailscale; daqui só dá pra perguntar ao backend até ficar pronto.
  function conferir() {
    pararConferencia();
    conferindo = true;
    const fim = Date.now() + CONFERIR_ATE_MS;
    const id = setInterval(async () => {
      if (Date.now() > fim) { pararConferencia(); return; }
      let r: SharePrereqs;
      try {
        r = await sharePrereqs(server());
      } catch {
        return;
      }
      if (timer !== id) return;
      preRequisito = r.missing.length ? r : null;
      if (!preRequisito) pararConferencia();
    }, CONFERIR_MS);
    timer = id;
  }

  async function gerar(local = false) {
    if (gerando) return;
    gerando = true;
    erroGerar = '';
    preRequisito = null;
    copiado = false;
    comandoCopiado = false;
    try {
      criado = await createShare(name, local, server());
      void carregar();
    } catch (e) {
      if (e instanceof SharePrerequisiteError) preRequisito = { missing: e.missing, fix: e.fix, enable_url: e.enableUrl };
      else erroGerar = msg(e);
    } finally {
      gerando = false;
    }
  }

  async function copiarComando() {
    if (!comandoOperador) return;
    await copyText(comandoOperador);
    comandoCopiado = true;
  }

  async function copiar() {
    if (!criado) return;
    await copyText(criado.link);
    copiado = true;
  }

  async function revogar(id: string) {
    try {
      await revokeShare(name, id, server());
      await carregar();
    } catch (e) {
      erroLista = msg(e);
    }
  }

  async function encerrarTodos() {
    try {
      await revokeAllShares(name, server());
      criado = null;
      await carregar();
    } catch (e) {
      erroLista = msg(e);
    }
  }

  const whatsapp = $derived(
    criado ? `https://wa.me/?text=${encodeURIComponent(m.compartilhar_whatsapp_texto({ link: criado.link }))}` : '',
  );
  const faltas = $derived(
    (preRequisito?.missing ?? []).map((f) =>
      f === 'operator' ? m.compartilhar_falta_operador() : f === 'funnel' ? m.compartilhar_falta_funnel() : f),
  );
  // Só a linha do comando: colar o `fix` inteiro num terminal rodaria também a frase do Funnel.
  const comandoOperador = $derived(
    preRequisito?.missing.includes('operator') ? preRequisito.fix.split('\n').find((l) => l.includes('--operator=')) ?? '' : '',
  );
</script>

<BottomSheet {open} {onClose} ariaLabel={m.compartilhar_titulo({ nome: name })} centered={desktop.atual}>
  <div class="share">
    <h2 class="share-titulo">{m.compartilhar_titulo({ nome: name })}</h2>
    <p class="share-aviso" role="note">{m.compartilhar_aviso_confianca()}</p>

    {#if criado}
      <div class="share-link">
        <span class="share-rotulo">{m.compartilhar_link_novo()}</span>
        <input class="share-url" readonly value={criado.link} aria-label={m.compartilhar_link_novo()}
               onfocus={(e) => e.currentTarget.select()} />
        <span class="share-sub">{m.compartilhar_vale_ate({ quando: resetsIn(criado.expires_at) })}</span>
        <div class="share-acoes">
          <button type="button" class="share-btn" onclick={copiar}>{copiado ? m.compartilhar_copiado() : m.compartilhar_copiar()}</button>
          <a class="share-btn primario" href={whatsapp} target="_blank" rel="noopener noreferrer">{m.compartilhar_whatsapp()}</a>
        </div>
      </div>
    {:else}
      <button type="button" class="share-btn primario" onclick={() => gerar()} disabled={gerando || conferindo}>
        {gerando ? m.compartilhar_gerando() : m.compartilhar_gerar()}
      </button>
      <button type="button" class="share-btn" onclick={() => gerar(true)} disabled={gerando || conferindo}>
        {m.compartilhar_gerar_local()}
      </button>
      <span class="share-sub">{m.compartilhar_gerar_local_dica()}</span>
    {/if}

    {#if preRequisito}
      <div class="share-erro" role="alert">
        <p>{m.compartilhar_pre_requisito()}</p>
        <ul>{#each faltas as f (f)}<li>{f}</li>{/each}</ul>
        {#if preRequisito.fix}<code class="share-fix">{preRequisito.fix}</code>{/if}
        <div class="share-acoes">
          {#if comandoOperador}
            <button type="button" class="share-btn" onclick={copiarComando}>{comandoCopiado ? m.compartilhar_copiado() : m.compartilhar_copiar()}</button>
          {/if}
          {#if preRequisito.enable_url}
            <a class="share-btn primario" href={preRequisito.enable_url} target="_blank" rel="noopener noreferrer"
               onclick={conferir}>{m.compartilhar_liberar_tailscale()}</a>
          {/if}
        </div>
        {#if comandoOperador}<p class="share-sub">{m.compartilhar_operador_dica()}</p>{/if}
        {#if conferindo}<p class="share-sub" role="status">{m.compartilhar_conferindo()}</p>{/if}
      </div>
    {:else if erroGerar}
      <p class="share-erro" role="alert">{erroGerar}</p>
    {/if}

    <h3 class="share-secao">{m.compartilhar_quem_entrou()}</h3>
    {#if erroLista}
      <p class="share-erro" role="alert">{m.compartilhar_erro_lista({ erro: erroLista })}</p>
      <button type="button" class="share-btn" onclick={carregar}>{m.compartilhar_tentar_de_novo()}</button>
    {:else if shares === null}
      <p class="share-sub" role="status">{m.compartilhar_carregando()}</p>
    {:else if shares.length === 0}
      <p class="share-sub">{m.compartilhar_vazio()}</p>
    {:else}
      <ul class="share-lista">
        {#each shares as s (s.id)}
          <li class="share-item">
            <span class="share-txt">
              <span class="share-nome">{s.pending ? m.compartilhar_link_aguardando() : (s.device ?? m.compartilhar_aparelho_sem_nome())}</span>
              <span class="share-sub">{s.pending
                ? m.compartilhar_pendente({ quando: resetsIn(s.expires_at) })
                : m.compartilhar_entrou({ quando: relativeTime(s.redeemed_at) })}</span>
            </span>
            <button type="button" class="share-btn perigo" onclick={() => revogar(s.id)}>{m.compartilhar_revogar()}</button>
          </li>
        {/each}
      </ul>
      <button type="button" class="share-btn perigo" onclick={encerrarTodos}>{m.compartilhar_encerrar_todos()}</button>
    {/if}
  </div>
</BottomSheet>

<style>
  .share { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-2) var(--space-4) var(--space-5); background: transparent; }
  .share-titulo { margin: 0; font-size: var(--text-base); font-weight: 600; color: var(--text-primary); }
  .share-aviso { margin: 0; font-size: var(--text-sm); color: var(--warning); }
  .share-link { display: flex; flex-direction: column; gap: var(--space-2); }
  .share-rotulo, .share-secao { margin: 0; font-size: var(--text-sm); font-weight: 600; color: var(--text-secondary); }
  .share-url {
    height: 36px; padding: 0 var(--space-3); font-family: var(--font-mono); font-size: var(--text-xs);
    color: var(--text-primary); background: var(--surface-inset); border: 1px solid var(--border-default); border-radius: var(--radius-sm);
  }
  .share-sub { font-size: var(--text-xs); color: var(--text-muted); }
  .share-acoes { display: flex; gap: var(--space-2); flex-wrap: wrap; }
  .share-btn {
    display: inline-flex; align-items: center; justify-content: center; height: 32px; padding: 0 var(--space-3);
    font-size: var(--text-sm); color: var(--text-primary); background: transparent; text-decoration: none;
    border: 1px solid var(--border-default); border-radius: var(--radius-sm); cursor: pointer;
  }
  .share-btn.primario { color: var(--accent); background: var(--accent-dim); border-color: transparent; font-weight: 600; }
  .share-btn.perigo { color: var(--error); }
  .share-btn:disabled { opacity: 0.5; cursor: default; }
  .share-erro { margin: 0; font-size: var(--text-sm); color: var(--error); }
  .share-erro p, .share-erro ul { margin: 0 0 var(--space-1); }
  .share-fix { display: block; padding: var(--space-2); font-size: var(--text-xs); background: var(--surface-inset); border-radius: var(--radius-sm); word-break: break-all; color: var(--text-primary); }
  .share-lista { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--space-1); }
  .share-item { display: flex; align-items: center; gap: var(--space-2); }
  .share-txt { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .share-nome { font-size: var(--text-sm); color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
