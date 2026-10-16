<script lang="ts">
  import InlineState from './InlineState.svelte';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ServerArt from './ServerArt.svelte';
  import ServerPlay from './ServerPlay.svelte';
  import OnlineCount from './OnlineCount.svelte';
  import { store } from '../lib/store.svelte';
  import { formatAddress } from '../lib/format';

  let { limit }: { limit: number } = $props();
</script>

<section aria-labelledby="featured-title">
  <header>
    <h2 id="featured-title">Featured servers</h2>
    {#if store.servers.status === 'ready' && store.servers.value.length > limit}
      <a href="#/servers" class="all" onclick={(e) => { e.preventDefault(); store.navigate('servers'); }}>
        All {store.servers.value.length}<ChevronRight size={14} strokeWidth={2} aria-hidden="true" />
      </a>
    {/if}
  </header>

  {#if store.servers.status === 'loading'}
    <ul aria-busy="true" aria-label="Loading servers">
      {#each { length: 3 } as _, i (i)}
        <li class="row"><span class="avatar"></span><span class="bar"></span></li>
      {/each}
    </ul>
  {:else if store.servers.status === 'error'}
    <InlineState message={store.offline ? 'You’re offline. Servers load when you reconnect.' : 'Featured servers didn’t load.'} onretry={() => void store.reloadServers()} />
  {:else if store.servers.value.length === 0}
    <InlineState message="No featured servers right now." />
  {:else}
    <ul>
      {#each store.servers.value.slice(0, limit) as server (server.id)}
        <li class="row">
          <span class="avatar"><ServerArt name={server.name} imageUrl={server.imageUrl} /></span>
          <span class="text">
            <span class="name">{server.name}</span>
            <span class="meta">
              {#if server.online !== undefined}<OnlineCount online={server.online} />{:else}{formatAddress(server.address, server.port)}{/if}
            </span>
          </span>
          <ServerPlay {server} />
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin-bottom: var(--s-2);
  }

  h2 {
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .all {
    display: inline-flex;
    align-items: center;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-2);
    text-decoration: none;
  }

  .all:hover {
    color: var(--text);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-2) 0;
  }

  .row + .row {
    border-top: 1px solid var(--line);
  }

  .avatar {
    flex: none;
    width: 52px;
    height: 52px;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--raised);
  }

  .text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .meta {
    font-size: var(--text-sm);
    color: var(--text-3);
  }

  .bar {
    width: 50%;
    height: 12px;
    border-radius: 4px;
    background: var(--raised);
  }

</style>
