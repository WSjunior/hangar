<script lang="ts">
  import { FieldSync, type PluginInputKind } from '@hangar/core';
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

  // Quando o valor desenhado entra no campo: a regra é o `FieldSync` do core, a mesma do nativo (pendente em foco, vez
  // da resposta ao envio, ecos velhos). Não é estado reativo: só o desenho novo, o blur e os handlers o leem.
  const sync = new FieldSync();
  let sendButton: HTMLButtonElement | undefined = $state();

  function put(next: string | null) {
    if (field && next !== null) field.value = next;
  }

  $effect(() => {
    const drawn = value;
    void frame;
    if (!field) return;
    put(sync.draw(drawn, field.value, document.activeElement === field));
  });

  function blur(e: FocusEvent) {
    // Tab até o rótulo de envio e Enter: o foco sai do campo antes do envio, e aplicar o pendente aqui faria o envio
    // mandar o valor do mod, e não o que está no campo. A resposta ao envio entra no desenho seguinte. (O clique no
    // rótulo não chega aqui: o `mousedown` dele não tira o foco do campo.)
    if (e.relatedTarget === sendButton) sync.leftToSend();
    else if (field) put(sync.draw(null, field.value, false));
  }

  function change(text: string) {
    if (!onInput) return;
    sync.typed(text);
    onInput('change', text);
  }

  function submit() {
    if (!field || !onInput) return;
    sync.submitted();
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
