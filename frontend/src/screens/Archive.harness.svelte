<script lang="ts">
  import Archive from './Archive.svelte';
  let { links }: { links: { serverId: string; project: string; sessionId: string }[] } = $props();
  let index = $state(0);
</script>

<button onclick={() => index = 1}>switch</button>
<Archive onBack={() => {}} deepLink={links[index]} />
