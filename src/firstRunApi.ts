// The first-run setup sheet's IPC (H6; Rust `first_run.rs`): its progress in
// config.json `setup`, one step marked done or skipped, and Run setup again.
// Its own module (calls keep api.ts's mock routing) so the App chunk carries
// none of it: only `firstRunMount.tsx`, the lazy sheet and Settings' Run
// setup again button import it.
import { invoke } from './api';

export type SetupStepId = 'context' | 'find' | 'sources' | 'agents' | 'start';
export type SetupStepState = 'open' | 'done' | 'skipped';

/** `first_run::SetupStep`. */
export interface SetupStep { id: SetupStepId; state: SetupStepState }

/** `first_run::SetupStatus`: no `setup` block in the config is complete. */
export interface SetupStatus { version: number; steps: SetupStep[]; current: SetupStepId | null; complete: boolean }

export const firstRunStatus = () => invoke<SetupStatus>('first_run_status');

/** The sheet's Continue (`done`) or Skip (`skipped`) on one step. */
export const firstRunSetStep = (step: SetupStepId, outcome: 'done' | 'skipped') =>
  invoke<SetupStatus>('first_run_set_step', { step, outcome });

/** Run setup again: every step open. */
export const firstRunReopen = () => invoke<SetupStatus>('first_run_reopen');
