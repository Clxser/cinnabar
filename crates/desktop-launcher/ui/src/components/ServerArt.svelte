<script lang="ts">
  import { bannerSvg, hashString } from '../lib/iso';

  interface Props {
    name: string;
    imageUrl?: string;
  }

  let { name, imageUrl }: Props = $props();
  let broken = $state(false);

  // Servers without artwork get a generated island tinted from their name, never an empty box.
  const generated = $derived(bannerSvg(name, hashString(name) % 360));
</script>

<div class="art">
  {#if imageUrl && !broken}
    <img src={imageUrl} alt="" loading="lazy" decoding="async" onerror={() => (broken = true)} />
  {:else}
    {@html generated}
  {/if}
</div>

<style>
  .art {
    width: 100%;
    height: 100%;
    background: var(--raised-2);
  }

  .art img,
  .art :global(svg) {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
