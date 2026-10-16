<script lang="ts">
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ServerCard from './ServerCard.svelte';
  import InlineState from './InlineState.svelte';
  import { store } from '../lib/store.svelte';

  interface Props {
    limit: number;
    variant: 'tile' | 'card';
    /** Lay cards out in a row instead of a stack. */
    row?: boolean;
  }

  let { limit, variant, row = false }: Props = $props();
</script>

<section class="shelf" aria-labelledby="featured-title">
  <header>
    <h2 id="featured-title">Featured servers</h2>
    {#if store.servers.status === 'ready' && store.servers.value.length > limit}
      <a href="#/servers" onclick={(e) => { e.preventDefault(); store.navigate('servers'); }}>
        All {store.servers.value.length}<ChevronRight size={15} strokeWidth={2} aria-hidden="true" />
      </a>
    {/if}
  </header>

  {#if store.servers.status === 'loading'}
    <ul class:row aria-busy="true" aria-label="Loading servers">
      {#each { length: limit } as _, i (i)}<li class="placeholder" data-variant={variant}></li>{/each}
    </ul>
  {:else if store.servers.status === 'error'}
    <InlineState message={store.offline ? 'You’re offline. Servers load when you reconnect.' : 'Featured servers didn’t load.'} onretry={() => void store.reloadServers()} />
  {:else if store.servers.value.length === 0}
    <InlineState message="No featured servers right now." />
  {:else}
    <ul class:row>
      {#each store.servers.value.slice(0, limit) as server (server.id)}
        <li><ServerCard {server} {variant} /></li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: var(--s-3);
  }

  h2 {
    font: var(--t-title);
  }

  a {
    display: inline-flex;
    align-items: center;
    font: var(--t-label);
    color: var(--ink-2);
    text-decoration: none;
  }

  a:hover {
    color: var(--ink);
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  ul.row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    gap: 16px;
  }

  .placeholder {
    height: 52px;
    border-radius: var(--r-md);
    background: var(--raise);
  }

  .placeholder[data-variant='card'] {
    height: auto;
    aspect-ratio: 16 / 10;
    border-radius: var(--r-lg);
  }
</style>
