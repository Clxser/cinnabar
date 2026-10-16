// The launcher bridge. Command names are the snake_case forms Rust registers; a Rust test
// checks that `tauri.ts` invokes exactly those commands and events.
import type {
  Channel,
  FeaturedServer,
  GameState,
  InstallState,
  LaunchOptions,
  News,
  UpdateState,
} from './types';

export interface Api {
  getInstallState(): Promise<InstallState>;
  getUpdateState(): Promise<UpdateState>;
  checkForUpdates(): Promise<void>;
  /** Downloads and installs the offered release; call only from an explicit click. */
  installUpdate(): Promise<void>;
  setChannel(channel: Channel): Promise<void>;
  launchGame(opts?: LaunchOptions): Promise<void>;
  getGameState(): Promise<GameState>;
  getNews(): Promise<News>;
  getFeaturedServers(): Promise<FeaturedServer[]>;
  openExternal(url: string): Promise<void>;
  /** Event `update-state`; returns the unsubscribe function. */
  onUpdateState(cb: (state: UpdateState) => void): () => void;
  /** Event `game-state`; returns the unsubscribe function. */
  onGameState(cb: (state: GameState) => void): () => void;
}
