<script lang="ts">
  import ArrowUpRight from '@lucide/svelte/icons/arrow-up-right';
  import InlineState from '../components/InlineState.svelte';
  import { store } from '../lib/store.svelte';
  import { CHANNEL_LABEL, checkStatus } from '../lib/play';
  import { DISCORD_URL, LICENSE_NAME, PRODUCT_NAME, SOURCE_URL } from '../lib/product';
  import type { Channel } from '../api';

  const CHANNEL_INFO: Record<Channel, string> = {
    stable: 'Release builds. The steadiest choice.',
    beta: 'Tested builds. Recommended for most players.',
    nightly: 'Builds from the latest changes, plus any newer Beta. Expect rough edges.',
  };

  const install = $derived(store.install.status === 'ready' ? store.install.value : undefined);
  // Channel order follows the contract's union; unpublished channels never render.
  const channels = $derived((['stable', 'beta', 'nightly'] as const).filter((c) => install?.publishedChannels.includes(c)));
  const busy = $derived(store.update?.kind === 'checking' || store.update?.kind === 'downloading' || store.update?.kind === 'verifying' || store.update?.kind === 'installing');
  const status = $derived(checkStatus(store.update, store.now));

  interface Notice {
    name: string;
    version: string;
    license: string;
    text: string;
  }

  // The build writes licenses.json from the modules it actually bundled.
  let licenses = $state<Notice[] | 'error'>();
  async function loadLicenses(e: Event) {
    if (!(e.currentTarget as HTMLDetailsElement).open || Array.isArray(licenses)) return;
    try {
      const res = await fetch('licenses.json');
      licenses = res.ok ? ((await res.json()) as Notice[]) : 'error';
    } catch {
      licenses = 'error';
    }
  }
</script>

<div class="page">
  <h1>Settings</h1>

  <section aria-labelledby="channel-title">
    <h2 id="channel-title">Release channel</h2>
    {#if store.install.status === 'error'}
      <InlineState message="Cinnabar couldn’t read its installation, so the channel can’t change." onretry={() => void store.reloadInstall()} />
    {:else if install}
      <div class="channels" role="radiogroup" aria-labelledby="channel-title">
        {#each channels as channel (channel)}
          <label class="channel" class:selected={install.channel === channel}>
            <input type="radio" name="channel" value={channel} checked={install.channel === channel} disabled={busy} onchange={() => void store.setChannel(channel)} />
            <span class="radio" aria-hidden="true"></span>
            <span class="copy">
              <span class="name">{CHANNEL_LABEL[channel]}</span>
              <span class="info">{CHANNEL_INFO[channel]}</span>
            </span>
          </label>
        {/each}
      </div>
      {#if install.channel !== install.installedChannel}
        <p class="note">
          The installed build is from {CHANNEL_LABEL[install.installedChannel]}. {CHANNEL_LABEL[install.channel]} applies with the next update.
        </p>
      {/if}
    {:else}
      <div class="channels" aria-busy="true"><span class="placeholder"></span><span class="placeholder"></span></div>
    {/if}
  </section>

  <section aria-labelledby="versions-title">
    <h2 id="versions-title">Versions</h2>
    <dl class="facts">
      <dt>Game</dt>
      <dd>{install ? `${install.version} (${install.build})` : '—'}</dd>
      <dt>Installed from</dt>
      <dd>{install ? CHANNEL_LABEL[install.installedChannel] : '—'}</dd>
      <dt>Launcher</dt>
      <dd>{install?.launcherVersion ?? '—'}</dd>
    </dl>
    <div class="row">
      <button class="button" disabled={busy || store.game?.kind !== 'stopped'} onclick={() => void store.check()}>Check for updates</button>
      {#if status}<span class="muted">{status}</span>{/if}
    </div>
  </section>

  <section aria-labelledby="about-title">
    <h2 id="about-title">About</h2>
    <p class="about">{PRODUCT_NAME} is an independent, source-available client for Bedrock servers. It is not affiliated with or endorsed by the makers of the original game.</p>
    <div class="links">
      <button class="link" onclick={() => void store.open(SOURCE_URL)}>Source code<ArrowUpRight size={14} strokeWidth={2} aria-hidden="true" /></button>
      <button class="link" onclick={() => void store.open(DISCORD_URL)}>Discord<ArrowUpRight size={14} strokeWidth={2} aria-hidden="true" /></button>
    </div>
    <p class="muted license">{PRODUCT_NAME} is licensed under {LICENSE_NAME}. Its display face, Cinnangles, is original to the project.</p>

    <details class="licenses" ontoggle={loadLicenses}>
      <summary>Third-party licenses in this launcher</summary>
      {#if licenses === 'error'}
        <InlineState message="The license list didn’t load." />
      {:else if licenses && licenses.length === 0}
        <InlineState message="Development builds don’t list licenses. Run a production build to see them." />
      {:else if licenses}
        <ul>
          {#each licenses as entry (entry.name)}
            <li>
              <details>
                <summary><span class="pkg">{entry.name}</span> <span class="muted">{entry.version} · {entry.license}</span></summary>
                <pre>{entry.text || 'This package ships no license file; its declared license is listed above.'}</pre>
              </details>
            </li>
          {/each}
        </ul>
      {/if}
    </details>
  </section>
</div>

<style>
  .page {
    max-width: 760px;
    padding: var(--s-6);
  }

  h1 {
    font-family: var(--font-display);
    font-size: var(--display-2);
    font-weight: 400;
    line-height: 1;
    text-transform: uppercase;
  }

  section {
    margin-top: var(--s-7);
  }

  section:first-of-type {
    margin-top: var(--s-6);
  }

  h2 {
    margin-bottom: var(--s-3);
    font-size: var(--text-lg);
    font-weight: 650;
  }

  .channels {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }

  .channel {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-4);
    border-radius: var(--r-md);
    background: var(--raised);
    cursor: pointer;
    transition: background-color var(--t-fast) linear, box-shadow var(--t-fast) linear;
  }

  .channel:hover {
    background: var(--raised-2);
  }

  .channel.selected {
    box-shadow: inset 0 0 0 1.5px var(--ore-hi);
  }

  .channel input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .channel:has(input:focus-visible) {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }

  .channel:has(input:disabled) {
    cursor: default;
    opacity: 0.6;
  }

  .radio {
    flex: none;
    width: 18px;
    height: 18px;
    margin-top: 1px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 2px var(--text-3);
  }

  .selected .radio {
    box-shadow: inset 0 0 0 5px var(--ore-hi);
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .name {
    font-weight: 600;
  }

  .info {
    color: var(--text-2);
    font-size: var(--text-sm);
  }

  .note {
    margin-top: var(--s-3);
    color: var(--warn);
    font-size: var(--text-sm);
  }

  .placeholder {
    height: 70px;
    border-radius: var(--r-md);
    background: var(--raised);
  }

  .facts {
    display: grid;
    grid-template-columns: 160px 1fr;
    row-gap: var(--s-2);
    margin: 0;
  }

  dt {
    color: var(--text-3);
  }

  dd {
    margin: 0;
    user-select: text;
    -webkit-user-select: text;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--s-4);
    margin-top: var(--s-4);
  }

  .button {
    height: 36px;
    padding: 0 var(--s-4);
    border-radius: var(--r-sm);
    background: var(--raised-2);
    font-weight: 600;
    transition: background-color var(--t-fast) linear;
  }

  .button:hover:not(:disabled) {
    background: var(--hover);
  }

  .button:disabled {
    color: var(--text-3);
  }

  .muted {
    color: var(--text-3);
    font-size: var(--text-sm);
  }

  .about {
    max-width: 62ch;
    color: var(--text-2);
  }

  .links {
    display: flex;
    gap: var(--s-5);
    margin-top: var(--s-3);
  }

  .link {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-weight: 600;
    text-decoration: underline;
    text-decoration-color: rgb(173 167 161 / 0.4);
    text-underline-offset: 3px;
  }

  .link:hover {
    text-decoration-color: currentColor;
  }

  .license {
    margin-top: var(--s-4);
  }

  .licenses {
    margin-top: var(--s-4);
  }

  summary {
    cursor: pointer;
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--text-2);
  }

  summary:hover {
    color: var(--text);
  }

  .licenses ul {
    margin: var(--s-3) 0 0;
    padding: 0;
    list-style: none;
  }

  .licenses li {
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
  }

  .pkg {
    color: var(--text);
  }

  pre {
    max-height: 240px;
    margin: var(--s-2) 0 0;
    padding: var(--s-3);
    overflow: auto;
    border-radius: var(--r-sm);
    background: var(--shell);
    color: var(--text-2);
    font-size: 11px;
    white-space: pre-wrap;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
