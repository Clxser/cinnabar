<script lang="ts">
  import InlineState from './InlineState.svelte';
  import ArrowUpRight from '@lucide/svelte/icons/arrow-up-right';
  import WifiOff from '@lucide/svelte/icons/wifi-off';
  import { store } from '../lib/store.svelte';
  import { formatAgo, formatDate } from '../lib/format';

  interface Props {
    limit: number;
  }

  let { limit }: Props = $props();
</script>

<section class="news" aria-labelledby="news-title">
  <header>
    <h2 id="news-title">News</h2>
    {#if store.news.status === 'ready' && store.news.value.stale}
      <p class="stale">
        <WifiOff size={13} strokeWidth={2} aria-hidden="true" />
        {store.news.value.fetchedAt ? `Saved ${formatAgo(store.news.value.fetchedAt, store.now)}` : 'Saved copy'}
      </p>
    {/if}
  </header>

  {#if store.news.status === 'loading'}
    <ul aria-busy="true" aria-label="Loading news">
      {#each { length: 3 } as _, i (i)}
        <li class="row placeholder"><span class="bar wide"></span><span class="bar"></span></li>
      {/each}
    </ul>
  {:else if store.news.status === 'error'}
    <InlineState message="News didn’t load. Playing isn’t affected." onretry={() => void store.reloadNews()} />
  {:else if store.news.value.items.length === 0}
    <InlineState message="Nothing new right now. Release notes appear here when a build ships." />
  {:else}
    <ul>
      {#each store.news.value.items.slice(0, limit) as item (item.id)}
        <li>
          <button class="row" onclick={() => void store.open(item.url)}>
            {#if item.imageUrl}<img class="thumb" src={item.imageUrl} alt="" loading="lazy" />{/if}
            <span class="text">
              <span class="title">{item.title}<ArrowUpRight class="out" size={14} strokeWidth={2} aria-hidden="true" /></span>
              <span class="summary">{item.summary}</span>
              <span class="meta">
                {#if item.tag}<span class="tag">{item.tag}</span>{/if}
                <time datetime={item.publishedAt}>{formatDate(item.publishedAt)}</time>
              </span>
            </span>
          </button>
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

  .stale {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--warn);
  }

  ul {
    margin: 0 calc(-1 * var(--s-3));
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    gap: var(--s-4);
    width: 100%;
    padding: var(--s-3);
    border-radius: var(--r-md);
    text-align: left;
    transition: background-color var(--t-fast) linear;
  }

  button.row:hover {
    background: var(--raised);
  }

  .thumb {
    flex: none;
    width: 112px;
    height: 63px;
    border-radius: var(--r-sm);
    object-fit: cover;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .title {
    display: flex;
    align-items: center;
    gap: 4px;
    font-weight: 600;
    color: var(--text);
  }

  .title :global(.out) {
    flex: none;
    color: var(--text-3);
    opacity: 0;
    transition: opacity var(--t-fast) linear;
  }

  button.row:hover .title :global(.out) {
    opacity: 1;
  }

  .summary {
    color: var(--text-2);
    font-size: var(--text-sm);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .meta {
    display: flex;
    gap: var(--s-2);
    margin-top: 2px;
    font-size: var(--text-xs);
    color: var(--text-3);
  }

  .tag {
    color: var(--ore-lit);
    font-weight: 600;
  }

  .placeholder {
    flex-direction: column;
    gap: var(--s-2);
  }

  .bar {
    display: block;
    width: 45%;
    height: 10px;
    border-radius: 4px;
    background: var(--raised);
  }

  .bar.wide {
    width: 80%;
    height: 12px;
  }

</style>
