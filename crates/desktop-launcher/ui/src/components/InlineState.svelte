<script lang="ts">
  interface Props {
    message: string;
    onretry?: () => void;
  }

  let { message, onretry }: Props = $props();
</script>

<div class="state" role={onretry ? 'alert' : undefined}>
  <p>{message}</p>
  {#if onretry}<button onclick={onretry}>Try again</button>{/if}
</div>

<style>
  .state {
    display: flex;
    align-items: center;
    gap: var(--s-4);
    padding: var(--s-4) 0;
    color: var(--text-2);
    font-size: var(--text-sm);
  }

  button {
    padding: 6px var(--s-3);
    border-radius: var(--r-sm);
    background: var(--raised-2);
    font-weight: 600;
    white-space: nowrap;
    transition: background-color var(--t-fast) linear;
  }

  button:hover {
    background: var(--hover);
  }
</style>
