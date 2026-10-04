<script lang="ts">
  // Só para teste: um chat (provide) com outro dentro (nested), como o modal da sessão do par.
  import { untrack } from 'svelte';
  import Self from './sessionServerProbe.fixture.svelte';
  import { parentSessionServerId, provideSessionServer } from './sessionServer';

  let { provide, depth = 0, report }: { provide?: string; depth?: number; report: (id: string | undefined) => void } = $props();
  untrack(() => {
    report(parentSessionServerId());
    if (provide) provideSessionServer(provide);
  });
</script>

{#if depth > 0}
  <Self depth={depth - 1} {report} />
{/if}
