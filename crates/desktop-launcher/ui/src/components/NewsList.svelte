<script lang="ts">
  import ArrowUpRight from '@lucide/svelte/icons/arrow-up-right';
  import WifiOff from '@lucide/svelte/icons/wifi-off';
  import InlineState from './InlineState.svelte';
  import { store } from '../lib/store.svelte';
  import { formatAgo, formatDate } from '../lib/format';

  interface Props {
    limit: number;
    /** `lead` sets the newest item as a headline above the rest. */
    variant?: 'rows' | 'lead';
    heading?: boolean;
  }

  let { limit, variant = 'rows', heading = true }: Props = $props();
  const items = $derived(store.news.status === 'ready' ? store.news.value.items.slice(0, limit) : []);
</script>

<section class="news" data-variant={variant} aria-labelledby={heading ? 'news-title' : undefined} aria-label={heading ? undefined : 'News'}>
  {#if heading}
    <header>
      <h2 id="news-title">News</h2>
      {#if store.news.status === 'ready' && store.news.value.stale}
        <p class="stale">
          <WifiOff size={13} strokeWidth={2} aria-hidden="true" />
          {store.news.value.fetchedAt ? `Saved ${formatAgo(store.news.value.fetchedAt, store.now)}` : 'Saved copy'}
        </p>
      {/if}
    </header>
  {/if}

  {#if store.news.status === 'loading'}
    <ul aria-busy="true" aria-label="Loading news">
      {#each { length: Math.min(limit, 3) } as _, i (i)}
        <li class="placeholder"><span class="bar wide"></span><span class="bar"></span></li>
      {/each}
    </ul>
  {:else if store.news.status === 'error'}
    <InlineState message="News didn’t load. Playing isn’t affected." onretry={() => void store.reloadNews()} />
  {:else if items.length === 0}
    <InlineState message="Nothing new right now. Release notes appear here when a build ships." />
  {:else}
    <ul>
      {#each items as item, i (item.id)}
        <li class:lead={variant === 'lead' && i === 0}>
          <button class="row" onclick={() => void store.open(item.url)}>
            {#if item.imageUrl}<img class="thumb" src={item.imageUrl} alt="" loading="lazy" />{/if}
            <span class="meta">
              {#if item.tag}<span class="tag">{item.tag}</span>{/if}
              <time datetime={item.publishedAt}>{formatDate(item.publishedAt)}</time>
            </span>
            <span class="title">{item.title}<ArrowUpRight class="out" size={15} strokeWidth={2} aria-hidden="true" /></span>
            {#if variant === 'lead' && i === 0}<span class="summary">{item.summary}</span>{/if}
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
    font: var(--t-title);
  }

  .stale {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font: var(--t-meta);
    color: var(--warn);
  }

  ul {
    margin: 0 -10px;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    padding: 10px;
    border-radius: var(--r-md);
    text-align: left;
    transition: background-color var(--t-fast) linear;
  }

  .row:hover {
    background: var(--raise);
  }

  .meta {
    display: flex;
    gap: 10px;
    font: var(--t-meta);
    color: var(--ink-3);
  }

  .tag {
    color: var(--cinnabar-hi);
    font-weight: 700;
  }

  .title {
    display: flex;
    align-items: baseline;
    gap: 4px;
    font: var(--t-label);
    font-size: 14px;
    color: var(--ink);
    text-wrap: pretty;
  }

  .lead .title {
    font: var(--t-title);
    font-size: 16px;
  }

  .summary {
    font: var(--t-meta);
    font-size: 13px;
    color: var(--ink-2);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .title :global(.out) {
    flex: none;
    align-self: center;
    color: var(--ink-3);
    opacity: 0;
    transform: translate(-3px, 3px);
    transition:
      opacity var(--t-fast) linear,
      transform var(--t-med) var(--ease-out);
  }

  .row:hover .title :global(.out) {
    opacity: 1;
    transform: none;
  }

  .thumb {
    width: 100%;
    aspect-ratio: 16 / 9;
    margin-bottom: 6px;
    border-radius: var(--r-sm);
    object-fit: cover;
  }

  .placeholder {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 10px;
  }

  .bar {
    display: block;
    width: 40%;
    height: 9px;
    border-radius: 4px;
    background: var(--raise);
  }

  .bar.wide {
    width: 85%;
    height: 12px;
  }
</style>
