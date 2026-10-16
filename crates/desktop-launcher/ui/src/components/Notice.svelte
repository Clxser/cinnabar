<script lang="ts">
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import type { Snippet } from 'svelte';

  interface Props {
    tone: 'warn' | 'error';
    title: string;
    children: Snippet;
    action?: { label: string; run: () => void };
  }

  let { tone, title, children, action }: Props = $props();
</script>

<div class="notice" data-tone={tone} role={tone === 'error' ? 'alert' : 'status'}>
  <span class="icon">
    {#if tone === 'error'}<CircleAlert size={18} strokeWidth={2} aria-hidden="true" />{:else}<TriangleAlert size={18} strokeWidth={2} aria-hidden="true" />{/if}
  </span>
  <div class="copy">
    <p class="title">{title}</p>
    <p class="body">{@render children()}</p>
  </div>
  {#if action}
    <button class="action" onclick={action.run}>{action.label}</button>
  {/if}
</div>

<style>
  .notice {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: start;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-md);
    background: var(--warn-wash);
  }

  .notice[data-tone='error'] {
    background: var(--bad-wash);
  }

  .icon {
    padding-top: 1px;
    color: var(--warn);
  }

  [data-tone='error'] .icon {
    color: var(--bad);
  }

  .title {
    font-weight: 600;
    color: var(--text);
  }

  .body {
    font-size: var(--text-sm);
    color: var(--text-2);
    text-wrap: pretty;
  }

  .action {
    align-self: center;
    padding: 7px var(--s-3);
    border-radius: var(--r-sm);
    background: var(--raised-2);
    font-size: var(--text-sm);
    font-weight: 600;
    white-space: nowrap;
    transition: background-color var(--t-fast) linear;
  }

  .action:hover {
    background: var(--hover);
  }
</style>
