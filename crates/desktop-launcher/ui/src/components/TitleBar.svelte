<script lang="ts">
  import Minus from '@lucide/svelte/icons/minus';
  import Square from '@lucide/svelte/icons/square';
  import X from '@lucide/svelte/icons/x';
  import WifiOff from '@lucide/svelte/icons/wifi-off';
  import mark from '../../../../../packaging/icons/cinnabar.svg';
  import { store } from '../lib/store.svelte';
  import { platform, windowAction } from '../lib/window';
  import { PRODUCT_NAME } from '../lib/product';
</script>

<header class="titlebar" data-tauri-drag-region>
  {#if platform !== 'macos'}
    <div class="brand" data-tauri-drag-region>
      <img src={mark} alt="" width="16" height="16" />
      <span data-tauri-drag-region>{PRODUCT_NAME}</span>
    </div>
  {/if}

  <div class="spacer" data-tauri-drag-region></div>

  {#if store.offline}
    <p class="offline" role="status">
      <WifiOff size={14} strokeWidth={2} aria-hidden="true" />
      Offline
    </p>
  {/if}

  {#if platform !== 'macos'}
    <div class="controls">
      <button aria-label="Minimize" onclick={() => windowAction('minimize')}><Minus size={16} strokeWidth={1.5} /></button>
      <button aria-label="Maximize" onclick={() => windowAction('toggleMaximize')}><Square size={12} strokeWidth={1.5} /></button>
      <button class="close" aria-label="Close" onclick={() => windowAction('close')}><X size={16} strokeWidth={1.5} /></button>
    </div>
  {/if}
</header>

<style>
  .titlebar {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    height: var(--titlebar);
    /* macOS draws its traffic lights over this inset. */
    padding-left: var(--s-4);
  }

  :global([data-platform='macos']) .titlebar {
    padding-left: 84px;
    padding-right: var(--s-4);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-2);
  }

  .brand img {
    display: block;
  }

  .spacer {
    flex: 1;
    align-self: stretch;
  }

  .offline {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--warn);
    margin-right: var(--s-3);
  }

  .controls {
    display: flex;
    align-self: stretch;
  }

  .controls button {
    display: grid;
    place-items: center;
    width: 46px;
    color: var(--text-2);
    transition: background-color var(--t-fast) linear;
  }

  .controls button:hover {
    background: var(--raised-2);
    color: var(--text);
  }

  .controls .close:hover {
    background: #c42b1c;
    color: #fff;
  }

  .controls button:focus-visible {
    outline-offset: -2px;
  }
</style>
