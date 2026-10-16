<script lang="ts">
  import { onMount } from 'svelte';
  import TitleBar from './components/TitleBar.svelte';
  import Rail from './components/Rail.svelte';
  import Home from './pages/Home.svelte';
  import Servers from './pages/Servers.svelte';
  import Settings from './pages/Settings.svelte';
  import { store } from './lib/store.svelte';
  import { platform } from './lib/window';

  onMount(() => store.start());

  // Native keyboard affordance: Mod+1..3 switch pages like tabs.
  function onKey(e: KeyboardEvent) {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    const page = ({ '1': 'home', '2': 'servers', '3': 'settings' } as const)[e.key as '1' | '2' | '3'];
    if (page) {
      e.preventDefault();
      store.navigate(page);
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="frame" data-platform={platform}>
  <TitleBar />
  <Rail />
  <main class="sheet" id="main" tabindex="-1">
    {#if store.page === 'servers'}
      <Servers />
    {:else if store.page === 'settings'}
      <Settings />
    {:else}
      <Home />
    {/if}
  </main>
</div>

<style>
  .frame {
    display: grid;
    grid-template-columns: var(--rail) 1fr;
    grid-template-rows: var(--titlebar) 1fr;
    height: 100%;
  }

  .sheet {
    grid-column: 2;
    grid-row: 2;
    background: var(--sheet);
    border-top-left-radius: var(--r-lg);
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    outline: none;
  }
</style>
