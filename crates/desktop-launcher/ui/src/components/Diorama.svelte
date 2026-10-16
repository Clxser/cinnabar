<script lang="ts">
  import { onMount } from 'svelte';
  import type { Diorama, Frame } from '../lib/diorama/scene';
  import { MOODS, moodForHour, type Mood } from '../lib/diorama/mood';
  // Rendered by this scene at dusk; shown when WebGL is unavailable.
  import still from '../assets/world-still.jpg';

  interface Props {
    /** Fraction of the cinnabar vein that glows. */
    refined: number;
    /** Stop rendering, e.g. while the game runs. */
    paused: boolean;
    /** Where the island sits within the canvas. */
    frame: Frame;
    /** Blur and dim for pages that sit over the world. */
    veiled?: boolean;
    orbit?: number;
  }

  let { refined, paused, frame, veiled = false, orbit }: Props = $props();

  const forced = new URLSearchParams(location.search).get('mood');
  const mood: Mood = forced && forced in MOODS ? (forced as Mood) : moodForHour(new Date().getHours());
  const m = MOODS[mood];
  const hex = (n: number) => `#${n.toString(16).padStart(6, '0')}`;

  let canvas = $state<HTMLCanvasElement>();
  let scene = $state<Diorama>();
  let failed = $state(false);
  let ready = $state(false);

  onMount(() => {
    const still = matchMedia('(prefers-reduced-motion: reduce)').matches || new URLSearchParams(location.search).has('still');
    let disposed = false;
    let instance: Diorama | undefined;
    const observer = new ResizeObserver(() => instance?.resize());
    const onPointer = (e: PointerEvent) => instance?.pointer((e.clientX / innerWidth) * 2 - 1, (e.clientY / innerHeight) * 2 - 1);

    import('../lib/diorama/scene')
      .then(({ createDiorama }) => {
        if (disposed) return;
        instance = createDiorama(canvas!, { mood, frame, still, orbit });
        scene = instance;
        observer.observe(canvas!);
        if (!still) addEventListener('pointermove', onPointer, { passive: true });
        requestAnimationFrame(() => (ready = true));
      })
      .catch((err: unknown) => {
        console.error('diorama unavailable', err);
        failed = true;
      });

    return () => {
      disposed = true;
      observer.disconnect();
      removeEventListener('pointermove', onPointer);
      instance?.dispose();
    };
  });

  $effect(() => scene?.setRefined(refined));
  $effect(() => scene?.setPaused(paused));
  $effect(() => scene?.setFrame(frame));
</script>

<div
  class="diorama"
  class:ready
  class:veiled
  style:--sky-top={hex(m.skyTop)}
  style:--sky-horizon={hex(m.horizon)}
  aria-hidden="true"
>
  {#if failed}
    <img src={still} alt="" />
  {:else}
    <canvas bind:this={canvas}></canvas>
  {/if}
</div>

<style>
  .diorama {
    position: absolute;
    inset: 0;
    overflow: hidden;
    /* The sky's own gradient shows while the scene loads and if WebGL is unavailable. */
    background: linear-gradient(to bottom, var(--sky-top), var(--sky-horizon));
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    opacity: 0;
    transition:
      opacity 900ms var(--ease-out),
      filter 500ms var(--ease-out),
      transform 500ms var(--ease-out);
  }

  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: 70% 40%;
    transition: filter 500ms var(--ease-out);
  }

  .veiled img {
    filter: blur(18px) brightness(0.55);
  }

  .ready canvas {
    opacity: 1;
  }

  .veiled canvas {
    filter: blur(18px) saturate(1.1) brightness(0.55);
    transform: scale(1.06);
  }

  @media (prefers-reduced-motion: reduce) {
    canvas {
      transition: none;
    }
  }
</style>
