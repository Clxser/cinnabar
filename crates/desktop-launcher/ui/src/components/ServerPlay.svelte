<script lang="ts">
  import Play from '@lucide/svelte/icons/play';
  import type { FeaturedServer } from '../api';
  import { store } from '../lib/store.svelte';
  import { joinBlockedReason } from '../lib/play';

  interface Props {
    server: FeaturedServer;
  }

  let { server }: Props = $props();

  const reason = $derived(joinBlockedReason(store.update, store.game));
</script>

<button class="join" disabled={reason !== undefined} title={reason} aria-label={`Play on ${server.name}`} onclick={() => void store.join(server)}>
  <Play size={14} strokeWidth={2.25} aria-hidden="true" />
  Play
</button>

<style>
  .join {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
    height: 32px;
    padding: 0 var(--s-3);
    border-radius: var(--r-sm);
    background: var(--raised-2);
    font-size: var(--text-sm);
    font-weight: 650;
    transition: background-color var(--t-fast) linear, color var(--t-fast) linear;
  }

  .join:hover:not(:disabled) {
    background: var(--ore);
    color: var(--ore-ink);
  }

  .join:disabled {
    color: var(--text-3);
  }
</style>
