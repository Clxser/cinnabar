<script lang="ts">
  import PlayCluster from '../components/PlayCluster.svelte';
  import NewsList from '../components/NewsList.svelte';
  import ServerShelf from '../components/ServerShelf.svelte';
  import { PRODUCT_NAME } from '../lib/product';
</script>

<!-- The world fills the window; launch floats on the left and one glass shelf runs along the foot. -->
<div class="home">
  <div class="hero">
    <h1>{PRODUCT_NAME}</h1>
    <PlayCluster />
  </div>
  <div class="shelf">
    <NewsList limit={2} variant="lead" />
    <ServerShelf limit={3} variant="card" row />
  </div>
</div>

<style>
  .home {
    display: grid;
    grid-template-rows: 1fr auto;
    height: 100%;
    padding: 0 40px 28px;
  }

  .hero {
    align-self: center;
    padding-bottom: 12px;
  }

  h1 {
    margin-bottom: 16px;
    font: var(--t-hero);
    font-stretch: 125%;
    letter-spacing: -0.025em;
  }

  .shelf {
    display: grid;
    grid-template-columns: minmax(250px, 0.85fr) minmax(0, 2.6fr);
    gap: 32px;
    padding: 20px 22px 22px 24px;
    border-radius: var(--r-lg);
    background: var(--glass);
    -webkit-backdrop-filter: blur(24px) saturate(1.5);
    backdrop-filter: blur(24px) saturate(1.5);
    box-shadow: inset 0 0 0 1px var(--glass-edge);
  }

  /* One orchestrated arrival: title, launch, then the shelf, each a beat apart. */
  @media (prefers-reduced-motion: no-preference) {
    h1,
    .hero :global(.cluster),
    .shelf {
      animation: rise 700ms var(--ease-out) both;
    }

    .hero :global(.cluster) {
      animation-delay: 70ms;
    }

    .shelf {
      animation-delay: 160ms;
    }
  }

  /* Short or narrow windows: a smaller title, a tighter shelf, and news reduced to its headline. */
  @media (max-height: 720px), (max-width: 1100px) {
    .home {
      padding: 0 28px 20px;
    }

    h1 {
      font-size: 40px;
      margin-bottom: 10px;
    }

    .shelf {
      grid-template-columns: minmax(220px, 0.8fr) minmax(0, 2fr);
      gap: 24px;
      padding: 16px 18px;
    }

    .shelf :global(.news .summary),
    .shelf :global(.news li + li) {
      display: none;
    }
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(14px);
    }
  }
</style>
