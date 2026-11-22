// Settings, Sources' typed IPC wrappers (overhaul H4a; backend
// `src-tauri/src/commands/sources_api.rs`, rules in `sources.rs`, contract
// docs/TRACKER_SYNC.md#sources-of-truth-h4a). Nothing imports this yet: the
// Sources screen (H4b) will, lazily, so none of it reaches the App chunk.
// Labels ("Tracked in Linear", "Stored in Hollow") come computed from Rust.

import { invoke } from './api';
import type {
  SourcesMigrationOutcome,
  SourcesMigrationPlan,
  SourcesMigrationRecord,
  SourcesMigrationStatus,
  SourcesMigrationTarget,
  SourcesStatus,
  SourcesUpdate,
} from './generated/ipc';

/** The three role rows, workspace overrides, knowledge references. */
export const sourcesStatus = () => invoke<SourcesStatus>('sources_status');
/** Save knowledge references; an owner change is refused (it is a switch). */
export const setSources = (update: Partial<SourcesUpdate>) => invoke<SourcesStatus>('sources_set', { update });
/** An owner switch's dry run: counts and files, nothing written. */
export const planSourcesSwitch = (target: SourcesMigrationTarget) =>
  invoke<SourcesMigrationPlan>('sources_migration_plan', { target });
/** Apply an owner switch (or finish one that stopped part way). */
export const applySourcesSwitch = (target: SourcesMigrationTarget) =>
  invoke<SourcesMigrationOutcome>('sources_migration_apply', { target });
/** Roll back one switch by its id, byte for byte. */
export const rollBackSourcesSwitch = (id: string) => invoke<SourcesMigrationRecord>('sources_migration_rollback', { id });
/** Every switch, newest first, and one that stopped part way. */
export const sourcesSwitches = () => invoke<SourcesMigrationStatus>('sources_migration_status');
