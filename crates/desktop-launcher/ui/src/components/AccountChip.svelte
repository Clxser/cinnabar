<script lang="ts">
  import LogIn from '@lucide/svelte/icons/log-in';
  import SkinHead from './SkinHead.svelte';
  import { store } from '../lib/store.svelte';
</script>

{#if store.account?.kind === 'signedOut'}
  <button class="chip signin" onclick={() => void store.signIn()}>
    <LogIn size={15} strokeWidth={2} aria-hidden="true" />
    Sign in
  </button>
{:else if store.account}
  <div class="chip" title={store.account.kind === 'offline' ? 'Signed in, but account services are unreachable' : undefined}>
    <SkinHead skinUrl={store.account.skinUrl} size={26} />
    <span class="who">
      <span class="name">{store.account.gamertag}</span>
      {#if store.account.kind === 'offline'}<span class="state">Offline</span>{/if}
    </span>
  </div>
{/if}

<style>
  .chip {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 14px 0 5px;
    border-radius: 999px;
    background: var(--glass);
    -webkit-backdrop-filter: blur(16px) saturate(1.4);
    backdrop-filter: blur(16px) saturate(1.4);
    box-shadow: inset 0 0 0 1px var(--glass-edge);
    font: var(--t-label);
    white-space: nowrap;
  }

  .signin {
    padding: 0 14px;
    gap: 8px;
    transition: background-color var(--t-fast) linear;
  }

  .signin:hover {
    background: var(--glass-strong);
  }

  .who {
    display: flex;
    flex-direction: column;
    line-height: 1.1;
  }

  .state {
    font: var(--t-meta);
    font-size: 11px;
    color: var(--warn);
  }
</style>
