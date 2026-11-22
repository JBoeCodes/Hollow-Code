/**
 * Paperclip PR 14a (cross-m34 M1): the ONE builder of the agent-settings
 * save payload, so no field (models, efforts, the no-output watchdogs, the
 * run deadlines) is silently reset by a save that forgot it.
 * Pure and dependency-free (vitest loads it in node); `import type` erases.
 */
import type { AgentSettings } from './types';
import { canvasesForSave, parseCanvasFrequency } from './canvasFrequency';

/** Paperclip PR 15b: the no-output watchdog's default and bounds, in
 *  minutes (Rust `agent_settings::NO_OUTPUT_DEFAULT_MIN` / `_MAX_MIN`). Every
 *  agent has one since merged plan slice 3. */
export const WATCHDOG_DEFAULT_MIN = 30;
export const WATCHDOG_MAX_MIN = 1440;
/** Merged plan slice 3: the largest per-run deadline (Rust
 *  `agent_settings::DEADLINE_MAX_MIN`). */
export const DEADLINE_MAX_MIN = 1440;

/** Trimmed, non-empty entries, keys sorted, so equal content compares equal
 *  whatever order it was edited in. */
function cleanMap(map: Record<string, string> | undefined): Record<string, string> {
  return Object.fromEntries(
    Object.entries(map ?? {})
      .map(([key, value]) => [key, (value ?? '').trim()] as const)
      .filter(([, value]) => value !== '')
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}

/** A watchdog as the backend stores it: absent → the default, `null` →
 *  off, a non-positive or unreadable number → the default, whole minutes up
 *  to the maximum. */
export function watchdogMinutes(value: number | null | undefined): number | null {
  if (value === null) return null;
  if (value === undefined || !Number.isFinite(value) || value <= 0) return WATCHDOG_DEFAULT_MIN;
  return Math.min(WATCHDOG_MAX_MIN, Math.max(1, Math.round(value)));
}

/** A per-run deadline as the backend stores it (merged plan slice 3):
 *  absent, `null`, non-positive or unreadable → none (`null`), else whole
 *  minutes up to the maximum. */
export function deadlineMinutes(value: number | null | undefined): number | null {
  if (value === null || value === undefined || !Number.isFinite(value) || value <= 0) return null;
  return Math.min(DEADLINE_MAX_MIN, Math.max(1, Math.round(value)));
}

/** `agent`'s watchdog in `settings`: Codex keeps its own field, every other
 *  agent its `noOutputTimeoutMin` entry, with the same semantics. */
export function agentWatchdog(settings: AgentSettings, agent: string): number | null {
  if (agent === 'codex') return watchdogMinutes(settings.codexNoOutputTimeoutMin);
  return watchdogMinutes(settings.noOutputTimeoutMin?.[agent]);
}

/** `settings` with `agent`'s watchdog set (`null` = off). */
export function withAgentWatchdog(settings: AgentSettings, agent: string, minutes: number | null): AgentSettings {
  if (agent === 'codex') return { ...settings, codexNoOutputTimeoutMin: minutes };
  return { ...settings, noOutputTimeoutMin: { ...(settings.noOutputTimeoutMin ?? {}), [agent]: minutes } };
}

/** `agent`'s per-run deadline in `settings` (`null` = none). */
export function agentDeadline(settings: AgentSettings, agent: string): number | null {
  return deadlineMinutes(settings.deadlineMin?.[agent]);
}

/** `settings` with `agent`'s deadline set (`null` = none). */
export function withAgentDeadline(settings: AgentSettings, agent: string, minutes: number | null): AgentSettings {
  const deadlineMin = { ...(settings.deadlineMin ?? {}) };
  if (minutes === null) delete deadlineMin[agent];
  else deadlineMin[agent] = minutes;
  return { ...settings, deadlineMin };
}

/** OB22: the compaction windows Hollow offers, in tokens (Rust
 *  `agent_settings::COMPACT_WINDOWS`). */
export const COMPACT_WINDOWS: readonly number[] = [300000, 400000, 600000];

/** OB22 review: a compaction window as a save sends it: one of
 *  {@link COMPACT_WINDOWS}, else `undefined` (off), so a hand-edited other
 *  size read back from the file is dropped rather than sent back. */
export function compactWindowForSave(value: number | null | undefined): number | undefined {
  return value != null && COMPACT_WINDOWS.includes(value) ? value : undefined;
}

function sortedEntries<T>(entries: [string, T][]): Record<string, T> {
  return Object.fromEntries(entries.sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));
}

/** What Settings saves from a draft. `efforts`, `noOutputTimeoutMin` and
 *  `deadlineMin` are left out when empty, `canvases` when it is When it
 *  helps, and a watchdog when it is the default, so a user who never set them saves exactly the payload they did
 *  before; a switched-off watchdog is sent as `null`. */
export function agentSettingsPayload(draft: AgentSettings): AgentSettings {
  const payload: AgentSettings = {
    defaultAgent: draft.defaultAgent,
    models: cleanMap(draft.models),
  };
  const efforts = cleanMap(draft.efforts);
  if (Object.keys(efforts).length > 0) payload.efforts = efforts;
  const watchdog = watchdogMinutes(draft.codexNoOutputTimeoutMin);
  if (watchdog !== WATCHDOG_DEFAULT_MIN) payload.codexNoOutputTimeoutMin = watchdog;
  const others = sortedEntries(
    Object.entries(draft.noOutputTimeoutMin ?? {})
      .filter(([agent]) => agent !== 'codex')
      .map(([agent, value]) => [agent, watchdogMinutes(value)] as [string, number | null])
      .filter(([, value]) => value !== WATCHDOG_DEFAULT_MIN),
  );
  if (Object.keys(others).length > 0) payload.noOutputTimeoutMin = others;
  const deadlines = sortedEntries(
    Object.entries(draft.deadlineMin ?? {})
      .map(([agent, value]) => [agent, deadlineMinutes(value)] as [string, number | null])
      .filter((entry): entry is [string, number] => entry[1] !== null),
  );
  if (Object.keys(deadlines).length > 0) payload.deadlineMin = deadlines;
  // Settings, Chat, Canvases (Canvas C3 wave 2 C4); the default is never sent.
  const canvases = canvasesForSave(parseCanvasFrequency(draft.canvases));
  if (canvases) payload.canvases = canvases;
  // OB22: the compaction window, which no control shows yet (S30), so a save
  // keeps it; off (absent) and a size Hollow doesn't offer are never sent.
  const compactWindow = compactWindowForSave(draft.compactWindow);
  if (compactWindow !== undefined) payload.compactWindow = compactWindow;
  // Repo ideas 2026-10-27 #8: When an agent runs out; Wait (absent) is
  // never sent.
  const whenOut = whenOutForSave(draft.whenOut);
  if (whenOut) payload.whenOut = whenOut;
  // The remote runner (2026-11-01): Runs on; this Mac (absent) is never sent.
  if (runsOnForSave(draft.runsOn)) payload.runsOn = RUNNER;
  // Overnight N6: Overnight and worker chats; absent (follow Runs on, the
  // default since M7) is never sent.
  const unattended = unattendedRunsOnForSave(draft.unattendedRunsOn);
  if (unattended) payload.unattendedRunsOn = unattended;
  // Smart routing 3: Builds and tests; absent (this Mac, the default since
  // M7) is never sent.
  const buildsOn = buildsOnForSave(draft.buildsOn);
  if (buildsOn) payload.buildsOn = buildsOn;
  // Smart routing 7: App builds; this Mac (absent) is never sent.
  if (appBuildsOnForSave(draft.appBuildsOn)) payload.appBuildsOn = RUNNER;
  // M3a PR2: COO on this Mac; auto (absent) is never sent.
  if (draft.cooDispatcher === 'off') payload.cooDispatcher = 'off';
  // OC1: Grok's ACP switch (`agents.acp.grok`), which no control shows yet,
  // so a save keeps it; off (absent) is never sent.
  if (draft.acp?.grok === true) payload.acp = { grok: true };
  return payload;
}

/** M6 (PER-20, 2026-11-15): the remote runner as a saved choice. Settings
 *  and workspace files from before it say `mac-mini`, which still means the
 *  same; a save sends `runner`. Rust `agent_settings::RUNNER`. */
export const RUNNER = 'runner';

/** Whether a saved value names the runner (`runner` or the old `mac-mini`).
 *  Rust `agent_settings::is_runner`. */
export function isRunner(value: string | null | undefined): boolean {
  const v = value?.trim();
  return v === RUNNER || v === 'mac-mini';
}

/** Smart routing 7 (2026-11-03): App builds as a save sends it: `runner`
 *  (from `runner` or the old `mac-mini`), or `undefined` for this Mac, the
 *  default (anything else). Rust `agent_settings::app_builds_on_choice`. */
export function appBuildsOnForSave(value: string | null | undefined): 'runner' | undefined {
  return isRunner(value) ? RUNNER : undefined;
}

/** Smart routing 3 (2026-11-03): Builds and tests as a save sends it:
 *  `runner` (from `runner` or the old `mac-mini`), `auto` or `this-mac`, or
 *  `undefined` for the default, this Mac since M7 (PER-21; anything else).
 *  Rust `agent_settings::builds_on_choice`. */
export function buildsOnForSave(value: string | null | undefined): 'runner' | 'auto' | 'this-mac' | undefined {
  if (isRunner(value)) return RUNNER;
  const v = value?.trim();
  return v === 'auto' || v === 'this-mac' ? v : undefined;
}

/** Overnight N6: Overnight and worker chats as a save sends it: `runner`
 *  (the runner when it answers; from `runner` or the old `mac-mini`),
 *  `same`, or `undefined` for the default, which follows Runs on like
 *  `same` since M7 (PER-21; anything else). Rust
 *  `agent_settings::unattended_runs_on_choice`. */
export function unattendedRunsOnForSave(value: string | null | undefined): 'runner' | 'same' | undefined {
  if (isRunner(value)) return RUNNER;
  return value?.trim() === 'same' ? 'same' : undefined;
}

/** Runs on as a save sends it: `runner` (from `runner` or the old
 *  `mac-mini`), or `undefined` for this Mac (the default; anything else).
 *  Rust `agent_settings::runs_on_choice`. */
export function runsOnForSave(value: string | null | undefined): 'runner' | undefined {
  return isRunner(value) ? RUNNER : undefined;
}

/** Repo ideas 2026-10-27 #8: the When an agent runs out choice as a save
 *  sends it: an agent id, or `undefined` for Wait (`wait`, nothing, or an id
 *  this build doesn't know). Rust `agent_settings::when_out_choice`. */
export function whenOutForSave(value: string | null | undefined): string | undefined {
  const id = value?.trim().toLowerCase();
  return id && ['claude', 'codex', 'antigravity', 'grok'].includes(id) ? id : undefined;
}

/** Settings, Models' draft of the loaded settings: every field,
 *  so an unedited draft compares equal to what was loaded (A4, RF-822-F1:
 *  leaving out `compactWindow` opened the pane dirty). */
export function draftFromSettings(value: AgentSettings): AgentSettings {
  return {
    defaultAgent: value.defaultAgent,
    models: { ...value.models },
    efforts: { ...(value.efforts ?? {}) },
    // Absent means the default (Paperclip PR 15b); null means off.
    codexNoOutputTimeoutMin: watchdogMinutes(value.codexNoOutputTimeoutMin),
    // Every other agent's watchdog and every agent's optional deadline
    // (merged plan slice 3), kept whole so a save never resets them.
    noOutputTimeoutMin: { ...(value.noOutputTimeoutMin ?? {}) },
    deadlineMin: { ...(value.deadlineMin ?? {}) },
    // Set elsewhere (Settings, Chat; `change_setting`); kept so the dirty
    // check matches.
    canvases: value.canvases,
    compactWindow: value.compactWindow,
    // Repo ideas 10-27 #8: the form's When an agent runs out row.
    whenOut: value.whenOut,
    // The remote runner: the form's Runs on row (as loaded; an old
    // `mac-mini` shows as the runner and saves as `runner`).
    runsOn: value.runsOn,
    // Overnight N6: the form's Overnight and worker chats row.
    unattendedRunsOn: value.unattendedRunsOn,
    // Smart routing 3: the form's Builds and tests row.
    buildsOn: value.buildsOn,
    // Smart routing 7: the form's App builds row.
    appBuildsOn: value.appBuildsOn,
    // M3a PR2: saved at once from Settings, Devices; kept so a save keeps it.
    cooDispatcher: value.cooDispatcher,
    // OC1: set by `change_setting` (`agents.acp.grok`); kept so a save keeps it.
    acp: value.acp ? { ...value.acp } : undefined,
  };
}

/** What Settings, Models saves: the draft, with the fields its
 *  Save form doesn't hold (Canvases, set from Settings, Chat; the
 *  compaction window, set by `change_setting`; efforts, which save on change
 *  from that pane and the composer alike, P0-19) taken from `live`, the file
 *  read just before the save, so the save neither drops nor reverts them. */
export function savePayload(draft: AgentSettings, live: AgentSettings): AgentSettings {
  return agentSettingsPayload({ ...draft, efforts: live.efforts, canvases: live.canvases, compactWindow: live.compactWindow, acp: live.acp });
}

/** Whether two settings would save the same payload (the dirty check).
 *  Efforts save on change, so they never make the Save form dirty. */
export function sameAgentSettings(a: AgentSettings, b: AgentSettings): boolean {
  const form = (s: AgentSettings) => JSON.stringify(agentSettingsPayload({ ...s, efforts: undefined }));
  return form(a) === form(b);
}

/** Settings S5b (2026-11-12): the agent defaults form, in four pieces that
 *  live in three sections (Models; Devices, the runner; Advanced, Run limits
 *  and App builds). Every field the Save form holds is in exactly one piece;
 *  efforts, Canvases and the compaction window save elsewhere and come from
 *  the live read (`savePayload`). */
export const AGENT_FORM_PIECES = {
  models: ['defaultAgent', 'whenOut', 'models'],
  runner: ['runsOn', 'buildsOn', 'unattendedRunsOn'],
  limits: ['codexNoOutputTimeoutMin', 'noOutputTimeoutMin', 'deadlineMin'],
  appBuilds: ['appBuildsOn'],
} as const satisfies Record<string, readonly (keyof AgentSettings)[]>;

export type AgentFormPiece = keyof typeof AGENT_FORM_PIECES;

/** `base` with `piece`'s fields taken from `draft`: what one piece's Save
 *  sends (through `savePayload`, the whole payload) and what its dirty
 *  check compares, so another piece's edits never count here. */
export function withPiece(base: AgentSettings, draft: AgentSettings, piece: AgentFormPiece): AgentSettings {
  const next: AgentSettings = { ...base };
  for (const field of AGENT_FORM_PIECES[piece]) (next as unknown as Record<string, unknown>)[field] = draft[field];
  return next;
}
