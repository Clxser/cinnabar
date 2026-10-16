<script lang="ts">
  import { oreField } from '../lib/iso';

  interface Props {
    /** Fraction of the vein drawn as refined ore, filled left to right. */
    fill: number;
  }

  let { fill }: Props = $props();

  const field = oreField(40);
  const lit = $derived(Math.round(Math.max(0, Math.min(1, fill)) * field.oreCount));

  const ROCK_L = { top: 27, right: 20, left: 14 } as const;
</script>

<svg class="field" viewBox="0 0 {field.width.toFixed(0)} {field.height.toFixed(0)}" aria-hidden="true" focusable="false">
  {#each field.cubes as cube, i (i)}
    {#if cube.ore < 0}
      {#each cube.faces as face (face.shade)}
        <polygon points={face.points} fill="hsl(222 7% {ROCK_L[face.shade] + cube.grain * 100 + (cube.depth - 0.5) * 6}%)" />
      {/each}
    {:else}
      <g class="ore" class:lit={cube.ore < lit}>
        {#each cube.faces as face (face.shade)}
          <polygon class={face.shade} points={face.points} />
        {/each}
      </g>
    {/if}
  {/each}
</svg>

<style>
  .field {
    display: block;
    width: 100%;
    height: auto;
    stroke: rgb(0 0 0 / 0.22);
    stroke-width: 0.75;
    stroke-linejoin: round;
  }

  .ore polygon {
    transition: fill var(--t-med) var(--ease-out);
  }

  .ore .top {
    fill: #4a2824;
  }

  .ore .right {
    fill: #3b211e;
  }

  .ore .left {
    fill: #2a1816;
  }

  .ore.lit .top {
    fill: var(--ore-lit);
  }

  .ore.lit .right {
    fill: var(--ore-hi);
  }

  .ore.lit .left {
    fill: var(--ore-deep);
  }

  @media (prefers-reduced-motion: reduce) {
    .ore polygon {
      transition: none;
    }
  }
</style>
