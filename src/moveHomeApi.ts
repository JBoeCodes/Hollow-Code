// Move Hollow home's IPC (2026-11-12; Rust `commands/move_home.rs`): the dry
// run, the run, the journal and rollback. Its own module (calls keep api.ts's
// mock routing), so the App chunk carries none of it: only the lazy Settings
// panel (`settings/MoveHomePanel.tsx`) imports it.
import { invoke } from './api';
import type { MoveHomeOptions, MoveHomePlan, MoveHomeRollback, MoveHomeStarted, MoveHomeStatus } from './types';

/** The dry run: reads, changes nothing. */
export const moveHomePlan = (options: MoveHomeOptions = {}) => invoke<MoveHomePlan>('move_home_plan', { options });
/** Stages the plan the person saw (its `hash`); Hollow quits right after. */
export const moveHomeStart = (hash: string, options: MoveHomeOptions = {}) => invoke<MoveHomeStarted>('move_home_start', { hash, options });
export const moveHomeStatus = () => invoke<MoveHomeStatus>('move_home_status');
/** Undoes the last move (or the one in `backupDir`), staged like the run. */
export const moveHomeRollback = (backupDir?: string | null) => invoke<MoveHomeRollback>('move_home_rollback', { backupDir: backupDir ?? null });
