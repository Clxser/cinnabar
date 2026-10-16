// Data the UI can show but the bridge contract does not carry yet. The launcher has no account
// core, so in Tauri these resolve to nothing and the UI hides what is missing.
import { inTauri } from './index';
import type { FeaturedServer } from './types';

export type Account =
  | { kind: 'signedIn'; gamertag: string; skinUrl?: string }
  | { kind: 'signedOut' }
  /** Cached identity while sign-in services are unreachable; the installed game still plays. */
  | { kind: 'offline'; gamertag: string; skinUrl?: string };

/** Optional fields proposed for `FeaturedServer`. */
export interface ServerExtras {
  motd?: string;
  tags?: string[];
  /** Round trip measured by the launcher, in milliseconds. */
  pingMs?: number;
}

export type Server = FeaturedServer & ServerExtras;

export async function getAccount(): Promise<Account | undefined> {
  if (inTauri) return undefined;
  const { mockAccount } = await import('./mock');
  return mockAccount();
}

export async function signIn(): Promise<void> {}
