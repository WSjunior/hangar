<script lang="ts">
  import CodexContaLogin from './CodexContaLogin.svelte';
  import type { Server } from '@hangar/core';
  let { servers, oncomplete }: { servers: Server[]; oncomplete: () => void } = $props();
  let index = $state(0);
</script>
<button onclick={() => index = 1}>switch</button>
<CodexContaLogin server={servers[index]} accountId="default" {oncomplete} />
