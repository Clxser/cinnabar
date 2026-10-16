<script lang="ts">
  import Launchpad from '../components/Launchpad.svelte';
  import NewsList from '../components/NewsList.svelte';
  import FeaturedServers from '../components/FeaturedServers.svelte';
</script>

<div class="home">
  <Launchpad />
  <div class="below">
    <NewsList limit={3} />
    <FeaturedServers limit={3} />
  </div>
</div>

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: var(--s-6);
  }

  .below {
    display: grid;
    grid-template-columns: minmax(0, 1.55fr) minmax(280px, 1fr);
    gap: var(--s-7);
  }

  @media (max-width: 960px) {
    .below {
      grid-template-columns: 1fr;
      gap: var(--s-6);
    }
  }
</style>
