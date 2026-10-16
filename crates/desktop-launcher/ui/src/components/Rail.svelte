<script lang="ts">
  import House from '@lucide/svelte/icons/house';
  import Globe from '@lucide/svelte/icons/globe';
  import Settings from '@lucide/svelte/icons/settings';
  import { store, type Page } from '../lib/store.svelte';

  const top = [
    { page: 'home', label: 'Home', icon: House },
    { page: 'servers', label: 'Servers', icon: Globe },
  ] as const;
</script>

{#snippet item(page: Page, label: string, Icon: typeof House)}
  <a
    href={page === 'home' ? '#' : `#/${page}`}
    class="item"
    aria-current={store.page === page ? 'page' : undefined}
    onclick={(e) => {
      e.preventDefault();
      store.navigate(page);
    }}
  >
    <span class="glyph"><Icon size={20} strokeWidth={1.75} aria-hidden="true" /></span>
    <span class="label">{label}</span>
  </a>
{/snippet}

<nav class="rail" aria-label="Main">
  <div class="group">
    {#each top as entry (entry.page)}
      {@render item(entry.page, entry.label, entry.icon)}
    {/each}
  </div>
  <div class="group">
    {@render item('settings', 'Settings', Settings)}
  </div>
</nav>

<style>
  .rail {
    grid-column: 1;
    grid-row: 2;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: var(--s-2) 0 var(--s-4);
  }

  .group {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-1);
  }

  .item {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    width: 64px;
    padding: var(--s-2) 0 7px;
    border-radius: var(--r-md);
    color: var(--text-3);
    text-decoration: none;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
    transition: color var(--t-fast) linear, background-color var(--t-fast) linear;
  }

  .glyph {
    display: grid;
    place-items: center;
    width: 40px;
    height: 30px;
    border-radius: var(--r-sm);
    transition: background-color var(--t-fast) linear;
  }

  .item:hover {
    color: var(--text);
  }

  .item:hover .glyph {
    background: var(--raised);
  }

  .item[aria-current='page'] {
    color: var(--text);
  }

  .item[aria-current='page'] .glyph {
    background: var(--raised-2);
    color: var(--ore-lit);
  }
</style>
