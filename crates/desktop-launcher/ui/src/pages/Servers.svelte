<script lang="ts">
  import Search from '@lucide/svelte/icons/search';
  import X from '@lucide/svelte/icons/x';
  import Copy from '@lucide/svelte/icons/copy';
  import Check from '@lucide/svelte/icons/check';
  import ServerArt from '../components/ServerArt.svelte';
  import ServerPlay from '../components/ServerPlay.svelte';
  import OnlineCount from '../components/OnlineCount.svelte';
  import InlineState from '../components/InlineState.svelte';
  import { store } from '../lib/store.svelte';
  import { formatAddress } from '../lib/format';
  import type { FeaturedServer } from '../api';

  let query = $state(new URLSearchParams(location.search).get('q') ?? '');
  let copied = $state<string>();

  const all = $derived(store.servers.status === 'ready' ? store.servers.value : []);
  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all.filter((s) => [s.name, s.description ?? '', s.address].some((field) => field.toLowerCase().includes(q)));
  });

  async function copyAddress(server: FeaturedServer) {
    try {
      await navigator.clipboard.writeText(formatAddress(server.address, server.port));
      copied = server.id;
      setTimeout(() => (copied = copied === server.id ? undefined : copied), 1600);
    } catch {
      copied = undefined;
    }
  }
</script>

<div class="page">
  <header class="head">
    <h1>Servers</h1>
    <label class="search">
      <Search size={16} strokeWidth={2} aria-hidden="true" />
      <span class="visually-hidden">Search featured servers</span>
      <input type="search" placeholder="Search by name or address" bind:value={query} spellcheck="false" autocomplete="off" />
      {#if query}
        <button class="clear" aria-label="Clear search" onclick={() => (query = '')}><X size={14} strokeWidth={2} /></button>
      {/if}
    </label>
  </header>
  <p class="lede">Play starts Cinnabar and joins the server.</p>

  {#if store.servers.status === 'loading'}
    <ul class="grid" aria-busy="true" aria-label="Loading servers">
      {#each { length: 6 } as _, i (i)}
        <li class="card"><span class="banner"></span><span class="bar"></span></li>
      {/each}
    </ul>
  {:else if store.servers.status === 'error'}
    <InlineState message={store.offline ? 'You’re offline. Featured servers load when you reconnect.' : 'Featured servers didn’t load.'} onretry={() => void store.reloadServers()} />
  {:else if all.length === 0}
    <InlineState message="No featured servers right now." />
  {:else if results.length === 0}
    <div class="none">
      <p>No servers match “{query.trim()}”.</p>
      <button class="text-button" onclick={() => (query = '')}>Clear search</button>
    </div>
  {:else}
    <p class="visually-hidden" aria-live="polite">{results.length} servers</p>
    <ul class="grid">
      {#each results as server (server.id)}
        <li class="card">
          <div class="banner"><ServerArt name={server.name} imageUrl={server.imageUrl} /></div>
          <div class="body">
            <h2>{server.name}</h2>
            {#if server.description}<p class="desc">{server.description}</p>{/if}
          </div>
          <div class="foot">
            <span class="facts">
              <OnlineCount online={server.online} />
              <button class="address" title="Copy address" onclick={() => copyAddress(server)}>
                {formatAddress(server.address, server.port)}
                {#if copied === server.id}<Check size={13} strokeWidth={2.5} aria-label="Copied" />{:else}<Copy size={13} strokeWidth={2} aria-hidden="true" />{/if}
              </button>
            </span>
            <ServerPlay {server} />
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .page {
    padding: var(--s-6);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-5);
  }

  h1 {
    font-family: var(--font-display);
    font-size: var(--display-2);
    font-weight: 400;
    line-height: 1;
    text-transform: uppercase;
  }

  .lede {
    margin-top: var(--s-3);
    color: var(--text-2);
  }

  .search {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: min(340px, 50%);
    height: 40px;
    padding: 0 var(--s-2) 0 var(--s-3);
    border-radius: var(--r-md);
    background: var(--raised);
    color: var(--text-3);
    transition: background-color var(--t-fast) linear;
  }

  .search:focus-within {
    background: var(--raised-2);
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: none;
    outline: none;
    color: var(--text);
    user-select: text;
    -webkit-user-select: text;
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .clear {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--r-sm);
    color: var(--text-2);
  }

  .clear:hover {
    background: var(--hover);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: var(--s-6) var(--s-5);
    margin: var(--s-5) 0 0;
    padding: 0;
    list-style: none;
  }

  .card {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .banner {
    display: block;
    aspect-ratio: 2 / 1;
    border-radius: var(--r-md);
    overflow: hidden;
    background: var(--raised);
  }

  .body {
    flex: 1;
    margin-top: var(--s-3);
  }

  h2 {
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .desc {
    margin-top: 2px;
    color: var(--text-2);
    font-size: var(--text-sm);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    margin-top: var(--s-3);
    font-size: var(--text-sm);
  }

  .facts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .address {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    color: var(--text-3);
    text-align: left;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .address:hover {
    color: var(--text);
  }

  .bar {
    display: block;
    width: 60%;
    height: 12px;
    margin-top: var(--s-3);
    border-radius: 4px;
    background: var(--raised);
  }

  .none {
    display: flex;
    align-items: center;
    gap: var(--s-4);
    margin-top: var(--s-6);
    color: var(--text-2);
  }

  .text-button {
    color: var(--text);
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
</style>
