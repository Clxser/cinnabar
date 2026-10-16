import type { Channel, GameState, InstallState, UpdateState } from '../api';
import { formatAgo, formatBytes } from './format';

export type PrimaryIntent = 'play' | 'update' | 'retry' | 'none';

export interface Primary {
  label: string;
  detail?: string;
  intent: PrimaryIntent;
  /** 0..1 fill while downloading; null for an indeterminate busy state. */
  progress?: number | null;
  tone: 'ore' | 'busy' | 'running' | 'disabled';
}

export interface Notice {
  tone: 'warn' | 'error';
  title: string;
  body: string;
  action?: { label: string; intent: 'retry' | 'logs' };
}

export type Stage = 'download' | 'verify' | 'install';

export interface PlayModel {
  primary: Primary;
  secondary?: { label: string; intent: 'playInstalled' };
  notice?: Notice;
  stage?: Stage;
}

export const CHANNEL_LABEL: Record<Channel, string> = { stable: 'Stable', beta: 'Beta', nightly: 'Nightly' };

/**
 * Derives what the launchpad offers from the three live states. Game state wins over update
 * state because an update cannot install while the game runs.
 */
export function playModel(install: InstallState | undefined, installFailed: boolean, update: UpdateState | undefined, game: GameState | undefined, now: number): PlayModel {
  if (installFailed) {
    return {
      primary: { label: 'Play', intent: 'none', tone: 'disabled' },
      notice: { tone: 'error', title: 'Cinnabar couldn’t read its installation', body: 'Reinstall Cinnabar from the website. Your worlds and settings are kept.' },
    };
  }
  if (!install || !game) return { primary: { label: 'Play', intent: 'none', tone: 'disabled' } };

  if (game.kind === 'starting') return { primary: { label: 'Starting', intent: 'none', tone: 'busy', progress: null } };
  if (game.kind === 'running') {
    return { primary: { label: 'Playing', detail: `Started ${formatAgo(game.startedAt, now)}`, intent: 'none', tone: 'running' } };
  }

  const crash: Notice | undefined = game.lastExit?.crashed
    ? {
        tone: 'warn',
        title: 'Cinnabar closed unexpectedly',
        body: game.lastExit.code === null ? 'The game stopped without an exit code.' : `The game stopped with exit code ${game.lastExit.code}.`,
        action: { label: 'View log', intent: 'logs' },
      }
    : undefined;
  const installed = `Version ${install.version}`;
  const play: Primary = { label: crash ? 'Play again' : 'Play', detail: installed, intent: 'play', tone: 'ore' };

  switch (update?.kind) {
    case 'downloading': {
      const total = update.totalBytes;
      const bytes = total ? `${formatBytes(update.receivedBytes)} of ${formatBytes(total)}` : `${formatBytes(update.receivedBytes)} so far`;
      return {
        primary: { label: 'Downloading', detail: `${update.version}, ${bytes}`, intent: 'none', tone: 'busy', progress: total ? update.receivedBytes / total : null },
        stage: 'download',
      };
    }
    case 'verifying':
      return { primary: { label: 'Verifying', detail: `Checking ${update.version} against its signature`, intent: 'none', tone: 'busy', progress: null }, stage: 'verify' };
    case 'installing':
      return { primary: { label: 'Installing', detail: `Replacing ${install.version} with ${update.version}`, intent: 'none', tone: 'busy', progress: null }, stage: 'install' };
    case 'available': {
      const size = update.sizeBytes ? `, ${formatBytes(update.sizeBytes)}` : '';
      const primary: Primary = { label: 'Update & play', detail: `Version ${update.version}${size}`, intent: 'update', tone: 'ore' };
      if (update.required) {
        return {
          primary,
          notice: { tone: 'warn', title: 'Update required', body: `Version ${install.version} can no longer be played. Install ${update.version} to keep playing.` },
        };
      }
      return { primary, secondary: { label: `Play ${install.version}`, intent: 'playInstalled' }, notice: crash };
    }
    case 'failed': {
      const retry = update.retryable ? { label: 'Try again', intent: 'retry' as const } : undefined;
      if (update.canPlayInstalled) {
        return { primary: play, notice: { tone: 'warn', title: 'Couldn’t update', body: update.message, action: retry } };
      }
      return {
        primary: update.retryable ? { label: 'Retry update', intent: 'retry', tone: 'ore' } : { label: 'Play', intent: 'none', tone: 'disabled' },
        notice: { tone: 'error', title: 'Cinnabar can’t start', body: update.message },
      };
    }
    default:
      return { primary: play, notice: crash };
  }
}

/** One line describing the last update check, or undefined while another surface explains it. */
export function checkStatus(update: UpdateState | undefined, now: number): string | undefined {
  switch (update?.kind) {
    case undefined:
      return undefined;
    case 'checking':
      return 'Checking for updates…';
    case 'upToDate':
      return `Up to date, checked ${formatAgo(update.lastChecked, now)}`;
    case 'idle':
      return update.lastChecked ? `Checked ${formatAgo(update.lastChecked, now)}` : 'Not checked for updates yet';
    default:
      return undefined;
  }
}

/** Why a server's Play button is unavailable, or undefined when joining can start now. */
export function joinBlockedReason(update: UpdateState | undefined, game: GameState | undefined): string | undefined {
  if (game && game.kind !== 'stopped') return 'Cinnabar is already running';
  switch (update?.kind) {
    case 'downloading':
    case 'verifying':
    case 'installing':
      return 'Wait for the update to finish';
    case 'available':
      return update.required ? 'Update Cinnabar to join' : undefined;
    case 'failed':
      return update.canPlayInstalled ? undefined : 'Cinnabar can’t start until the update is fixed';
    default:
      return undefined;
  }
}

/** How much of the diorama's vein glows: the download's progress, dark when the build can't start. */
export function veinFill(model: PlayModel): number {
  if (model.stage === 'download') return model.primary.progress ?? 0;
  if (model.notice?.tone === 'error') return 0;
  return 1;
}
