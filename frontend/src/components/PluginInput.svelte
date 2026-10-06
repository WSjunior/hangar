<script lang="ts">
  import type { PluginInputKind } from '@hangar/core';
  import * as m from '../paraglide/messages';

  interface Props {
    label: string;
    placeholder: string;
    /** Valor que o mod desenhou. */
    value: string;
    /** O que o Enter faz; vazio, o rótulo do app. */
    submitLabel: string;
    /** Sem ele (sessão com terminal, ou servidor que não diz a fonte) o campo fica desabilitado, com a dica. */
    onInput?: (kind: PluginInputKind, value: string) => void;
    /** Identidade do desenho do mod (o nó da árvore): muda a cada evento novo, mesmo com o mesmo `value`. */
    frame?: unknown;
  }
  let { label, placeholder, value, submitLabel, onInput, frame }: Props = $props();
  let field: HTMLInputElement | undefined = $state();

  // Quando o valor desenhado entra, como no nativo: só conta como posto quando é posto. Com a pessoa no campo ele fica
  // pendente (um redesenho atrasado não apaga o que se digita) e entra quando o campo perde o foco. Logo depois do
  // envio do próprio campo, o desenho seguinte entra mesmo com foco: é como o mod limpa o campo depois do envio, e o
  // valor pode ser igual ao de antes (vazio). Digitar depois que o pendente chegou o descarta: perder o foco nunca apaga
  // texto digitado e não enviado. Não são estado reativo: só o desenho novo e o blur as leem.
  let pending: string | null = null;
  let submitted = false;
  let sendButton: HTMLButtonElement | undefined = $state();

  function put(drawn: string) {
    if (field && field.value !== drawn) field.value = drawn;
  }

  $effect(() => {
    const drawn = value;
    void frame;
    if (!field) return;
    // Depois do envio, um desenho igual ao que se vê (o eco da última tecla) não é a resposta ao envio: a vez fica.
    if (document.activeElement !== field || (submitted && drawn !== field.value)) {
      pending = null;
      submitted = false;
      put(drawn);
    } else {
      pending = drawn;
    }
  });

  function blur(e: FocusEvent) {
    // Tab até o rótulo de envio e Enter: o foco sai do campo antes do envio, e aplicar o pendente aqui faria o envio
    // mandar o valor do mod, e não o que está no campo. A resposta ao envio entra no desenho seguinte. (O clique no
    // rótulo não chega aqui: o `mousedown` dele não tira o foco do campo.)
    if (pending !== null && e.relatedTarget !== sendButton) put(pending);
    pending = null;
  }

  function change(text: string) {
    // Voltar a digitar descarta o pendente e fecha a vez do desenho que responde ao envio.
    pending = null;
    submitted = false;
    onInput?.('change', text);
  }

  function submit() {
    if (!field || !onInput) return;
    // O que foi enviado é o que está no campo: um eco de antes do envio não volta ao perder o foco.
    pending = null;
    submitted = true;
    onInput('submit', field.value);
  }
</script>

<span class="plugin-field">
  {#if label}<span class="label">{label}</span>{/if}
  <input bind:this={field} type="text" {placeholder} aria-label={label || placeholder} disabled={!onInput}
         oninput={(e) => change(e.currentTarget.value)} onblur={blur}
         onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); submit(); } }} />
  {#if onInput}
    <!-- O `preventDefault` no `mousedown` (que o toque também gera, antes do click) deixa o foco no campo, como no
         nativo: no Safari e no Firefox do macOS o botão não recebe o foco, e o blur do campo aplicaria o pendente antes
         do envio; com o campo em foco, um redesenho entre o mousedown e o click também fica pendente. O click continua. -->
    <button bind:this={sendButton} type="button" class="submit" onmousedown={(e) => e.preventDefault()} onclick={submit}>{submitLabel || m.plugin_input_enviar()}</button>
  {:else}
    <span class="hint">{m.plugin_input_no_terminal()}</span>
  {/if}
</span>

<style>
  /* Classe própria: a `.field` global do app.css é coluna com margem, e empilhava rótulo, campo e envio. Aqui é
     uma linha, como no terminal; sem espaço, o envio quebra para baixo e o campo não fica menor que 12ch. */
  .plugin-field { display: inline-flex; flex-direction: row; align-items: center; flex-wrap: wrap; gap: 1ch; min-width: 0; }
  .label { white-space: nowrap; }
  input { font: inherit; color: inherit; background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: var(--radius-sm); padding: 0 0.5ch; min-width: 12ch; flex: 1 1 16ch; }
  input:disabled { opacity: 0.6; }
  .submit { font: inherit; color: var(--accent); background: transparent; border: 0; padding: 0; cursor: pointer; min-height: 0; min-width: 0; }
  .hint { color: var(--text-muted); font-family: var(--font-ui); }
</style>
