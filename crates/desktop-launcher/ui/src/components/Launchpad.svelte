<script lang="ts">
  import ArrowUpRight from '@lucide/svelte/icons/arrow-up-right';
  import Check from '@lucide/svelte/icons/check';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import OreField from './OreField.svelte';
  import PlayButton from './PlayButton.svelte';
  import Notice from './Notice.svelte';
  import { store } from '../lib/store.svelte';
  import { CHANNEL_LABEL, checkStatus, playModel, type Stage } from '../lib/play';
  import { PRODUCT_NAME } from '../lib/product';

  const install = $derived(store.install.status === 'ready' ? store.install.value : undefined);
  const model = $derived(playModel(install, store.install.status === 'error', store.update, store.game, store.now));
  const status = $derived(checkStatus(store.update, store.now));
  const notes = $derived(store.update?.kind === 'available' ? store.update : undefined);

  const STAGES: { id: Stage; label: string }[] = [
    { id: 'download', label: 'Download' },
    { id: 'verify', label: 'Verify' },
    { id: 'install', label: 'Install' },
  ];
  const stageIndex = $derived(model.stage ? STAGES.findIndex((s) => s.id === model.stage) : -1);

  // The vein mirrors the install: it refines as bytes arrive and dims when the build cannot start.
  const fill = $derived.by(() => {
    const p = model.primary;
    if (model.stage === 'download') return p.progress ?? 0;
    if (model.notice?.tone === 'error') return 0;
    return 1;
  });

  function activate() {
    const intent = model.primary.intent;
    if (intent === 'play') void store.play();
    else if (intent === 'update') void store.updateAndPlay();
    else if (intent === 'retry') void store.retry();
  }
</script>

<section class="launchpad" aria-labelledby="launchpad-title">
  <div class="art">
    <OreField {fill} />
  </div>

  <div class="content">
    <h1 id="launchpad-title" class="wordmark">{PRODUCT_NAME}</h1>
    <p class="identity">
      {#if install}
        {CHANNEL_LABEL[install.installedChannel]} {install.version}
        <span class="build">build {install.build}</span>
      {:else if store.install.status === 'loading'}
        <span class="skeleton" aria-label="Loading version"></span>
      {:else}
        Version unknown
      {/if}
    </p>

    {#if model.notice}
      {@const notice = model.notice}
      <div class="notice">
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
              {#if i < stageIndex}<Check size={13} strokeWidth={2.5} aria-hidden="true" />{/if}
              {stage.label}
            </li>
          {/each}
        </ol>
      {:else if notes}
        {#if notes.notesUrl}
          <button class="link" onclick={() => void store.open(notes.notesUrl!)}>
            What’s new in {notes.version}<ArrowUpRight size={14} strokeWidth={2} aria-hidden="true" />
          </button>
        {/if}
      {:else if status && store.game?.kind === 'stopped'}
        <span class="check-line">
          {#if store.update?.kind === 'checking'}<LoaderCircle class="spin" size={14} strokeWidth={2} aria-hidden="true" />{/if}
          {status}
        </span>
        {#if store.update?.kind !== 'checking'}
          <button class="link" onclick={() => void store.check()}>Check now</button>
        {/if}
      {/if}
    </div>
  </div>
</section>

<style>
  .launchpad {
    position: relative;
    display: flex;
    min-height: 368px;
    border-radius: var(--r-lg);
    background: var(--raised);
    overflow: hidden;
    isolation: isolate;
  }

  .art {
    position: absolute;
    z-index: -1;
    top: 20px;
    right: -24px;
    width: min(600px, 54%);
    /* Let the outcrop emerge from the panel instead of ending on a hard cut. */
    mask-image: linear-gradient(100deg, transparent 4%, #000 30%);
    -webkit-mask-image: linear-gradient(100deg, transparent 4%, #000 30%);
  }

  .content {
    display: flex;
    flex-direction: column;
    justify-content: center;
    width: min(500px, 100%);
    padding: var(--s-6) var(--s-6) var(--s-6) 40px;
  }

  .wordmark {
    font-family: var(--font-display);
    font-size: var(--display-3);
    font-weight: 400;
    line-height: 1;
    text-transform: uppercase;
    letter-spacing: 0;
  }

  .identity {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 20px;
    margin-top: var(--s-3);
    color: var(--text-2);
    font-weight: 500;
  }

  .build {
    color: var(--text-3);
    font-weight: 400;
  }

  .skeleton {
    width: 140px;
    height: 12px;
    border-radius: 4px;
    background: var(--raised-2);
  }

  .notice {
    max-width: 420px;
    margin-top: var(--s-5);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-5);
    margin-top: var(--s-5);
  }

  .secondary {
    padding: var(--s-2) 0;
    color: var(--text-2);
    font-weight: 600;
    text-decoration: underline;
    text-decoration-color: transparent;
    text-underline-offset: 4px;
    transition: color var(--t-fast) linear, text-decoration-color var(--t-fast) linear;
  }

  .secondary:hover {
    color: var(--text);
    text-decoration-color: currentColor;
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: 20px;
    margin-top: var(--s-4);
    font-size: var(--text-sm);
    color: var(--text-3);
  }

  .check-line {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--text-2);
    font-weight: 600;
    text-decoration: underline;
    text-decoration-color: rgb(173 167 161 / 0.4);
    text-underline-offset: 3px;
  }

  .link:hover {
    color: var(--text);
    text-decoration-color: currentColor;
  }

  .stages {
    display: flex;
    gap: var(--s-4);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .stages li {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .stages .done {
    color: var(--text-2);
  }

  .stages .current {
    color: var(--ore-lit);
    font-weight: 600;
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
