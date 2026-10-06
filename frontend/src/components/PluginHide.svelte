<script lang="ts">
  import * as m from '../paraglide/messages';

  // Atalho do celular para esconder a interface dos mods ali mesmo: desliga a preferência do menu "⋯", que continua
  // sendo o caminho para ligar de novo.
  let { onHide }: { onHide: () => void } = $props();
</script>

<button type="button" class="plugin-hide" aria-label={m.mods_celular_ocultar()} onclick={onHide}>{m.mods_celular_ocultar_curto()}</button>

<style>
  /* Pequeno no desenho, com 32 px de alvo de toque: o mínimo global de 44 px dobraria a altura da faixa. */
  .plugin-hide {
    flex: none;
    min-height: 32px;
    min-width: 32px;
    padding: 0 var(--space-2);
    font-family: var(--font-ui);
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--surface-raised);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }
</style>
