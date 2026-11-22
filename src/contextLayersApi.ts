// Settings, Context's IPC (CX17; Rust `context_settings.rs` and the sync
// engine's own commands in `sync_engine.rs`). Its own module (calls keep
// api.ts's mock routing), so the App chunk carries none of it: only the lazy
// panel (`settings/ContextLayersPanel.tsx`) imports it.
import { invoke } from './api';

/** The sync engine's status for one root (`sync_engine::SyncStatus`). */
export interface ContextSyncStatus {
  root: string;
  state: 'off' | 'idle' | 'syncing' | 'blocked' | 'offline' | 'clash';
  enabled: boolean;
  /** Any user part shown as `***`. */
  remote: string | null;
  pendingCommits: number;
  unsyncedFiles: number;
  /** The pill's one line ("Up to date", "2 changes not synced"). */
  text: string;
  lastSyncAt: string | null;
  lastError: string | null;
  blockedBy: string | null;
  clashCopies: string[];
}

/** A context clone folder (`context_roots::RootStatus`). */
export interface ContextRootStatus {
  layer: 'personal' | 'org';
  /** The saved folder, else the suggested default. */
  path: string;
  configured: boolean;
  exists: boolean;
  gitClone: boolean;
  lastCommitAt: string | null;
  problem: string | null;
}

/** One synced folder's two sides (`context_settings::SeedFolder`, PX1). */
export interface ContextSeedFolder {
  folder: string;
  onlyVault: string[];
  onlyClone: string[];
  differ: string[];
  same: number;
  capped: boolean;
}

/** What the personal sync's first cycle would do (`context_settings::SeedReport`). */
export interface ContextSeedReport { clone: string; folders: ContextSeedFolder[]; text: string }

export interface ContextLayersStatus {
  home: { slug: string | null; suggestedSlug: string | null; file: string | null; fileExists: boolean; root: ContextRootStatus };
  personal: ContextSyncStatus;
  org: { root: ContextRootStatus; remote: string | null; lastPullAt: string | null };
  projects: { withContext: { title: string; path: string; files: string[]; git: boolean }[]; total: number; capped: boolean; error: string | null };
  vault: { setUp: boolean; enabled: boolean; remote: string | null; text: string };
  /** The first sync's dry run: only before any sync has run and with a personal clone (PX1). */
  seed?: ContextSeedReport | null;
}

export interface OrgAccessCheck {
  state: 'ok' | 'denied' | 'offline' | 'missing' | 'error';
  text: string;
}

export const contextLayersStatus = () => invoke<ContextLayersStatus>('context_layers_status');
/** `allowDeletions` only after the person confirms the removals the guard stopped (one sync). */
export const contextSyncNow = (allowDeletions = false) => invoke<ContextSyncStatus>('context_sync_now', { allowDeletions });
export const setContextSyncEnabled = (enabled: boolean) => invoke<ContextSyncStatus>('set_context_sync_enabled', { enabled });
/** The backend refuses a URL with credentials in it; nothing secret is saved. */
export const configurePersonalSync = (remote: string) => invoke<ContextSyncStatus>('configure_personal_sync', { remote });
export const contextPersonalLocalOnly = () => invoke<ContextSyncStatus>('context_personal_local_only');
export const contextOrgAccessCheck = () => invoke<OrgAccessCheck>('context_org_access_check');
export const setContextRoot = (layer: 'personal' | 'org', path: string | null) => invoke<ContextRootStatus>('set_context_root', { layer, path });
export const setContextMachineSlug = (slug: string | null) => invoke<string | null>('set_context_machine_slug', { slug });

/** A repo a pre-push context check can guard (`leak_checks::CheckTarget`, CX10). */
export interface ContextCheckRepo {
  title: string;
  path: string;
  layer: 'org' | 'personal' | 'vault' | 'project';
  state: 'installed' | 'not_installed' | 'other_hook' | 'unavailable';
  problem: string | null;
}

/** Settings, Context's context check rows (`leak_checks::CheckStatus`). */
export interface ContextCheckStatus { mode: 'flag' | 'block'; repos: ContextCheckRepo[] }

/** What an install or remove did (`leak_checks::InstallOutcome` / `RemoveOutcome`). */
export interface ContextCheckOutcome { applied: boolean; text: string }

export const contextCheckStatus = () => invoke<ContextCheckStatus>('context_check_status');
/** The person's click: writes the hook (the agent action defaults to a dry run). */
export const contextInstallCheck = (path: string) => invoke<ContextCheckOutcome>('context_install_check', { path, apply: true });
export const contextRemoveCheck = (path: string) => invoke<ContextCheckOutcome>('context_remove_check', { path, apply: true });

/** One row of "Connect this Mac's context" (F3, `mac_context::LayerRow`). */
export interface MacContextLayer {
  layer: 'personal' | 'org' | 'home' | 'vault';
  state: 'connected' | 'missing' | 'held';
  text: string;
  /** Where a clone goes (the saved root, else the default). */
  target: string | null;
  /** A repo address this Mac already knows (no credentials). */
  url: string | null;
}

/** The card (`mac_context::MacContextOffer`); `open` shows Not now and Not for this Mac. */
export interface MacContextOffer {
  pair: string | null;
  otherName: string | null;
  otherKind: 'runner' | 'peer' | null;
  open: boolean;
  layers: MacContextLayer[];
  slug: string | null;
  suggestedSlug: string | null;
}

/** One layer's result (`mac_context::Step`). */
export interface MacContextStep { layer: string; state: 'done' | 'skipped' | 'held' | 'failed' | 'refused'; text: string }
export interface MacContextReport { steps: MacContextStep[]; connected: boolean }
export interface MacContextRequest { pair: string | null; layers: string[]; personalUrl: string | null; orgUrl: string | null; slug: string | null }

/** `any`: the rows whatever the answer, so Settings can still connect later. */
export const macContextOffer = (any = false) => invoke<MacContextOffer | null>('mac_context_offer', { any });
export const macContextAnswer = (pair: string, answer: 'not-now' | 'never') => invoke<void>('mac_context_answer', { pair, answer });
export const macContextConnect = (request: MacContextRequest) => invoke<MacContextReport>('mac_context_connect', { request });
