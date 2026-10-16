// Placeholder copy of the launcher API contract; the Tauri builder owns the final file.

export type Channel = 'stable' | 'beta' | 'nightly';

export interface InstallState {
  version: string;            // installed game version, e.g. "0.1.21"
  build: string;              // short commit or build id
  channel: Channel;           // desired channel (persisted)
  installedChannel: Channel;  // channel the installed payload came from
  publishedChannels: Channel[]; // only channels with real releases
  launcherVersion: string;
}

export type UpdateState =
  | { kind: 'idle'; lastChecked?: string }
  | { kind: 'checking' }
  | { kind: 'upToDate'; lastChecked: string }
  | { kind: 'available'; version: string; channel: Channel; notesUrl?: string; sizeBytes?: number; required: boolean }
  | { kind: 'downloading'; version: string; receivedBytes: number; totalBytes?: number }
  | { kind: 'verifying'; version: string }
  | { kind: 'installing'; version: string }
  | { kind: 'failed'; message: string; retryable: boolean; canPlayInstalled: boolean };

export type GameState =
  | { kind: 'stopped'; lastExit?: { code: number | null; crashed: boolean } }
  | { kind: 'starting' }
  | { kind: 'running'; startedAt: string };

export interface NewsItem { id: string; title: string; summary: string; url: string; imageUrl?: string; publishedAt: string; tag?: string }
export interface News { items: NewsItem[]; stale: boolean; fetchedAt?: string }

export interface FeaturedServer { id: string; name: string; address: string; port: number; description?: string; imageUrl?: string; online?: number }

export interface Api {
  getInstallState(): Promise<InstallState>;
  getUpdateState(): Promise<UpdateState>;
  checkForUpdates(): Promise<void>;
  installUpdate(): Promise<void>;          // explicit click only
  setChannel(channel: Channel): Promise<void>;
  launchGame(opts?: { server?: { address: string; port: number } }): Promise<void>;
  getGameState(): Promise<GameState>;
  getNews(): Promise<News>;
  getFeaturedServers(): Promise<FeaturedServer[]>;
  openExternal(url: string): Promise<void>;
  onUpdateState(cb: (s: UpdateState) => void): () => void;  // event 'update-state'
  onGameState(cb: (s: GameState) => void): () => void;      // event 'game-state'
}
