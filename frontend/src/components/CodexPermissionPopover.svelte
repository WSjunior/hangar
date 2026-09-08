<script lang="ts">
  // Pílula de permissão de uma sessão Codex — irmã de ClaudePermissionPopover e de
  // CodexEffortPopover. Não reusa a do Claude porque o dado é de outra natureza: lá os modos são
  // um conjunto fechado com rótulo e descrição traduzidos aqui; aqui rótulo e descrição vêm do
  // picker VIVO do Codex (`/permissions`), em inglês, e são dado do agente — como nome de sessão
  // ou saída de comando, não viram chave de idioma. Um quarto modo aparece na tela sozinho.
  import * as m from '../paraglide/messages';
  import Popover from './Popover.svelte';
  import { getCodexPermissions, setCodexPermission, type CodexPermissionMode } from '../lib/api';

  interface Props {
    open: boolean;
    anchor: HTMLElement | null;
    sessionName: string;
    onApplied: (modo: string) => void;
    onClose: () => void;
  }
  let { open, anchor, sessionName, onApplied, onClose }: Props = $props();

  let modos = $state<CodexPermissionMode[]>([]);
  let atual = $state<string | null>(null);
  let carregando = $state(false);
  let aplicando = $state<string | null>(null);
  let err = $state<string | null>(null);

  // Relê a cada abertura: o modo também muda pelo terminal, e uma lista de minutos atrás mostraria
  // o tique no modo errado.
  $effect(() => {
    if (!open) return;
    const sn = sessionName;
    err = null;
    aplicando = null;
    carregando = true;
    getCodexPermissions(sn)
      .then((res) => {
        if (sn !== sessionName) return;
        modos = res.modes;
        atual = res.current;
        // A pílula também vive do que a LEITURA achou: alimentada só pelo `onApplied` da troca,
        // ela ficava no rótulo genérico até o usuário mudar de modo — inclusive na sessão que já
        // estava em Full Access.
        if (res.current) onApplied(res.current);
      })
      .catch((e) => {
        if (sn !== sessionName) return;
        err = e instanceof Error ? e.message : m.comum_falha_aplicar();
      })
      .finally(() => { if (sn === sessionName) carregando = false; });
  });

  async function escolher(modo: string) {
    if (aplicando) return;
    if (modo === atual) { onClose(); return; }
    aplicando = modo;
    err = null;
    try {
      // O backend devolve o que FICOU (relê o `(current)` do picker), não o que foi pedido.
      const res = await setCodexPermission(sessionName, modo);
      atual = res.current;
      onApplied(res.current);
    } catch (e) {
      err = e instanceof Error ? e.message : m.comum_falha_aplicar();
      aplicando = null;
      return;
    }
    aplicando = null;
    onClose();
  }
</script>

<Popover {open} {anchor} {onClose} width={260} ariaLabel={m.composer_permissao()}>
  {#if err}
    <p class="err" role="alert">{err}</p>
  {/if}

  {#if carregando}
    <p class="vazio">{m.comum_carregando()}</p>
  {:else if modos.length === 0}
    <p class="vazio">{m.permissao_codex_sem_lista()}</p>
  {:else}
    <ul class="lista">
      {#each modos as modo (modo.numero)}
        {@const ativo = atual === modo.nome}
        <li>
          <button
            class="linha"
            class:ativa={ativo}
            aria-pressed={ativo}
            disabled={!!aplicando}
            data-foco={ativo ? true : undefined}
            onclick={() => escolher(modo.nome)}
          >
            <span class="nome">
              <span class="rotulo">{modo.nome}</span>
              <span class="desc">{modo.desc}</span>
            </span>
            {#if aplicando === modo.nome}
              <span class="tick" aria-hidden="true">…</span>
            {:else if ativo}
              <svg class="tick" width="16" height="16" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="2.5" stroke-linecap="round"
                stroke-linejoin="round" aria-hidden="true">
                <polyline points="20 6 9 17 4 12" />
              </svg>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</Popover>

<style>
  .err { color: var(--error); font-size: var(--text-xs); margin: 8px 10px 0; }
  .vazio { color: var(--text-muted); font-size: var(--text-sm); text-align: center; padding: 14px 0; }
  .lista { list-style: none; margin: 0; padding: 4px 0; overflow-y: auto; }

  .linha {
    display: flex; align-items: flex-start; gap: 8px; width: 100%;
    padding: 8px 10px; background: transparent; border: none;
    color: var(--text-primary); font-size: var(--text-sm); text-align: left; cursor: pointer;
  }
  .linha:hover:not(:disabled) { background: var(--bg-hover); }
  .linha:disabled { cursor: default; }
  .linha.ativa .rotulo { font-weight: 600; }

  .nome { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .desc { color: var(--text-muted); font-size: var(--text-xs); line-height: 1.35; }
  .tick { flex-shrink: 0; color: var(--accent); margin-top: 2px; }
</style>
