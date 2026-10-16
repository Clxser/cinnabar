<script lang="ts">
  import ArrowUpRight from '@lucide/svelte/icons/arrow-up-right';
  import Check from '@lucide/svelte/icons/check';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import { slide } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import PlayButton from './PlayButton.svelte';
  import Notice from './Notice.svelte';
  import { store } from '../lib/store.svelte';
  import { CHANNEL_LABEL, checkStatus, playModel, type PlayModel, type Stage } from '../lib/play';

  interface Props {
    align?: 'start' | 'center';
  }

  let { align = 'start' }: Props = $props();

  const install = $derived(store.install.status === 'ready' ? store.install.value : undefined);
  const model: PlayModel = $derived(playModel(install, store.install.status === 'error', store.update, store.game, store.now));
  const status = $derived(checkStatus(store.update, store.now));
  const notes = $derived(store.update?.kind === 'available' ? store.update : undefined);
  const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const reveal = (node: Element) => slide(node, { duration: reduce ? 0 : 280, easing: cubicOut });

  const STAGES: { id: Stage; label: string }[] = [
    { id: 'download', label: 'Download' },
    { id: 'verify', label: 'Verify' },
    { id: 'install', label: 'Install' },
  ];
  const stageIndex = $derived(STAGES.findIndex((s) => s.id === model.stage));

  function activate() {
    const intent = model.primary.intent;
    if (intent === 'play') void store.play();
    else if (intent === 'update') void store.updateAndPlay();
    else if (intent === 'retry') void store.retry();
  }
</script>

<div class="cluster" data-align={align}>
  <p class="identity">
    {#if install}
      <span class="channel">{CHANNEL_LABEL[install.installedChannel]}</span>
      <span>Version {install.version}</span>
      <span class="build">{install.build}</span>
    {:else if store.install.status === 'loading'}
      <span class="skeleton" aria-label="Loading version"></span>
    {:else}
      <span>Version unknown</span>
    {/if}
  </p>

  {#if model.notice}
    {@const notice = model.notice}
    <div class="notice" transition:reveal>
      <Notice tone={notice.tone} title={notice.title} action={notice.action ? { label: notice.action.label, run: () => void store.retry() } : undefined}>
        {notice.body}
      </Notice>
    </div>
  {/if}

  <div class="actions">
    <PlayButton primary={model.primary} onactivate={activate} />
    {#if model.secondary}
      <button class="secondary" onclick={() => void store.play()}>{model.secondary.label}</button>
    {/if}
  </div>

  <div class="status" aria-live="polite">
    {#if model.stage}
      <ol class="stages" aria-label="Update progress">
        {#each STAGES as stage, i (stage.id)}
          <li class:done={i < stageIndex} class:current={i === stageIndex} aria-current={i === stageIndex ? 'step' : undefined}>
            <span class="tick">{#if i < stageIndex}<Check size={11} strokeWidth={3} aria-hidden="true" />{/if}</span>
            {stage.label}
          </li>
        {/each}
      </ol>
    {:else if notes?.notesUrl}
      <button class="link" onclick={() => void store.open(notes.notesUrl!)}>
        What’s new in {notes.version}<ArrowUpRight size={14} strokeWidth={2} aria-hidden="true" />
      </button>
    {:else if status && install && store.game?.kind === 'stopped'}
      <span class="check">
        {#if store.update?.kind === 'checking'}<LoaderCircle class="spin" size={14} strokeWidth={2} aria-hidden="true" />{/if}
        {status}
      </span>
      {#if store.update?.kind !== 'checking'}
        <button class="link" onclick={() => void store.check()}>Check now</button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .cluster {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }

  .cluster[data-align='center'] {
    align-items: center;
    text-align: center;
  }

  .identity {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 24px;
    font: var(--t-label);
    color: var(--ink-2);
  }

  .channel {
    padding: 3px 9px;
    border-radius: 999px;
    background: rgb(226 72 58 / 0.18);
    color: var(--cinnabar-hi);
    font-weight: 700;
  }

  .build {
    color: var(--ink-3);
    font-weight: 500;
  }

  .skeleton {
    width: 180px;
    height: 12px;
    border-radius: 4px;
    background: var(--raise-2);
  }

  .notice {
    width: 100%;
    max-width: 440px;
    margin-top: var(--s-4);
    text-align: left;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-5);
    margin-top: var(--s-5);
  }

  [data-align='center'] .actions {
    flex-direction: column;
    gap: var(--s-3);
  }

  .secondary {
    padding: var(--s-2) 0;
    font: var(--t-label);
    font-size: 14px;
    color: var(--ink-2);
    text-decoration: underline;
    text-decoration-color: transparent;
    text-underline-offset: 4px;
    transition: color var(--t-fast) linear, text-decoration-color var(--t-fast) linear;
  }

  .secondary:hover {
    color: var(--ink);
    text-decoration-color: currentColor;
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: 22px;
    margin-top: var(--s-4);
    font: var(--t-meta);
    font-size: 13px;
    color: var(--ink-3);
  }

  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--ink-2);
    font-weight: 600;
    text-decoration: underline;
    text-decoration-color: rgb(201 194 187 / 0.4);
    text-underline-offset: 3px;
  }

  .link:hover {
    color: var(--ink);
    text-decoration-color: currentColor;
  }

  .stages {
    display: flex;
    gap: var(--s-5);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .stages li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .tick {
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1.5px currentColor;
  }

  .done {
    color: var(--ink-2);
  }

  .done .tick {
    background: var(--ink-2);
    color: var(--solid);
    box-shadow: none;
  }

  .current {
    color: var(--ink);
    font-weight: 700;
  }

  .current .tick {
    box-shadow: inset 0 0 0 4px var(--cinnabar);
  }

  :global(.spin) {
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.spin) {
      animation: none;
    }
  }
</style>
