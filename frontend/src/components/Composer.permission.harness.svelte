<script lang="ts">
  import Composer from './Composer.svelte';
  import type { State } from '../lib/types';

  let sessionState = $state<State>('working');
  let permissionMode = $state<string | null>(null);
  let previousNonPlan = $state<string | null>(null);
  // exposto pro teste chamar
  export function setState(v: State) { sessionState = v; }
  export function getState() { return sessionState; }
  export function setPermission(mode: string, previous: string) {
    permissionMode = mode;
    previousNonPlan = previous;
  }
</script>

<Composer
  sessionName="perm-test"
  {sessionState}
  claudePermissionMode={permissionMode}
  claudePreviousNonPlan={previousNonPlan}
  status={null}
  onSend={async () => {}}
  onCommand={() => {}}
  onInterrupt={() => {}}
  onOpenGit={() => {}}
  onOpenPreview={() => {}}
/>
