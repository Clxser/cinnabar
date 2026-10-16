<script lang="ts">
  import type { Primary } from '../lib/play';

  interface Props {
    primary: Primary;
    onactivate: () => void;
  }

  let { primary, onactivate }: Props = $props();

  const percent = $derived(typeof primary.progress === 'number' ? Math.floor(primary.progress * 100) : undefined);
</script>

<button
  class="play"
  data-tone={primary.tone}
  disabled={primary.intent === 'none'}
  aria-busy={primary.tone === 'busy'}
  onclick={onactivate}
>
  {#if primary.tone === 'busy'}
    <span class="track" class:indeterminate={percent === undefined} style:--p={percent === undefined ? 1 : percent / 100}></span>
  {/if}
  <span class="face">
    <span class="label">
      {#if primary.tone === 'running'}<span class="dot" aria-hidden="true"></span>{/if}
      {primary.label}{#if percent !== undefined}<span class="pct">{percent}%</span>{/if}
    </span>
    {#if primary.detail}<span class="detail">{primary.detail}</span>{/if}
  </span>
</button>

<style>
  .play {
    position: relative;
    display: block;
    width: 320px;
    height: 76px;
    padding: 0 0 4px;
    border-radius: var(--r-md);
    overflow: hidden;
    text-align: left;
    color: var(--ore-ink);
    background: var(--raised-2);
    /* The pressable edge borrows the in-game OreUI button: a darker lower face that compresses on press. */
    box-shadow: inset 0 -4px 0 rgb(0 0 0 / 0.28);
    transition:
      background-color var(--t-fast) linear,
      box-shadow var(--t-fast) var(--ease-out);
  }

  .face {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
    height: 100%;
    padding: 0 var(--s-5);
    transition: transform var(--t-fast) var(--ease-out);
  }

  .label {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    font-family: var(--font-display);
    font-size: var(--display-2);
    line-height: 1;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .pct {
    margin-left: auto;
    font-family: var(--font-ui);
    font-size: var(--text-lg);
    font-weight: 600;
  }

  .detail {
    font-size: var(--text-sm);
    font-weight: 500;
    opacity: 0.86;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .play[data-tone='ore'] {
    background: var(--ore);
    box-shadow:
      inset 0 -4px 0 var(--ore-deep),
      inset 0 1px 0 rgb(255 255 255 / 0.16);
  }

  .play[data-tone='ore']:hover {
    background: var(--ore-hi);
  }

  .play[data-tone='ore']:active {
    box-shadow:
      inset 0 -1px 0 var(--ore-deep),
      inset 0 1px 0 rgb(255 255 255 / 0.16);
  }

  .play[data-tone='ore']:active .face {
    transform: translateY(2px);
  }

  .play[data-tone='busy'] {
    background: var(--ore-dormant);
    box-shadow: inset 0 -4px 0 #24110f;
  }

  .track {
    position: absolute;
    inset: 0 0 4px;
    background: var(--ore);
    transform-origin: left;
    transform: scaleX(var(--p));
    transition: transform 400ms var(--ease-out);
  }

  /* Indeterminate work: one solid block of ore travelling the track. */
  .track.indeterminate {
    width: 28%;
    transform: none;
    animation: travel 1.5s cubic-bezier(0.65, 0, 0.35, 1) infinite alternate;
  }

  @keyframes travel {
    from {
      translate: 0 0;
    }
    to {
      translate: calc(100% / 0.28 - 100%) 0;
    }
  }

  .play[data-tone='running'] {
    color: var(--text);
  }

  .play[data-tone='running'] .detail {
    color: var(--text-2);
    opacity: 1;
  }

  .dot {
    width: 10px;
    height: 10px;
    border-radius: 2px;
    background: var(--ok);
  }

  .play[data-tone='disabled'] {
    color: var(--text-3);
  }

  .play:focus-visible {
    outline-offset: 3px;
  }

  @media (prefers-reduced-motion: reduce) {
    .track.indeterminate {
      width: 100%;
      animation: none;
      opacity: 0.35;
    }

    .track,
    .face {
      transition: none;
    }
  }
</style>
