// Settings, Sources' words (overhaul H4b with two-Macs T5; design
// `sources-and-import-2026-11-13` section 3). The three role rows (Tasks and
// projects, Knowledge, Personal context), the tasks row's two choices (T5:
// Vault only, or Vault plus Linear), a switch's dry run read as plain lines,
// and the label tone every item uses. The labels themselves ("Tracked in
// Linear", "Stored in Hollow") come computed from Rust (`sources::label`),
// so the phone and the desktop never disagree. Pure; unit-tested. Loaded
// with the lazy Sources panel and the Tasks page, never the App chunk.
import type {
  SourceLabel, SourceLabelKind, SourcesBinding, SourcesMigrationPlan, SourcesOwnerKind, SourcesReference, SourcesRole, SourcesRoleRow,
  SourcesWorkspaceRow,
} from './generated/ipc';

export const ROLE_TITLES: Record<SourcesRole, string> = {
  tasks: 'Tasks and projects',
  knowledge: 'Knowledge',
  personal: 'Personal context',
};

/** Personal context is not a setting: nothing can move `me/` or `learnings/`. */
export const PERSONAL_LINE = 'Always in your Hollow folder';

/** The tracker T5 offers by name until a second provider exists. */
export const DEFAULT_TRACKER = 'Linear';

/** The tasks row's two choices (T5). T4 (Notion as a task source) is gone. */
export function taskChoices(providerName?: string | null): { value: SourcesOwnerKind; label: string; hint: string }[] {
  const name = providerName || DEFAULT_TRACKER;
  return [
    { value: 'vault', label: 'Vault only', hint: 'Tasks and projects live in your Hollow folder.' },
    { value: 'tracker', label: `Vault plus ${name}`, hint: `${name} owns its fields; a copy of each task stays in your vault.` },
  ];
}

export type LabelTone = 'ok' | 'off' | 'warn';

/** The chip state a label shows with (`settings-state-chip` data-state). */
export function labelTone(kind: SourceLabelKind): LabelTone {
  if (kind === 'unreachable') return 'warn';
  if (kind === 'readOnly') return 'off';
  return 'ok';
}

/** "Inherits: Linear", "Override: vault only", "Override: Linear". */
export function bindingLine(binding: SourcesBinding): string {
  const where = binding.owner === 'tracker' ? (binding.providerName || DEFAULT_TRACKER) : 'vault only';
  return binding.inherited ? `Inherits: ${where}` : `Override: ${where}`;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** Edits waiting, held and in conflict on a row, or null when there are none. */
export function waitingLine(row: Pick<SourcesRoleRow, 'queuedEdits' | 'heldEdits' | 'conflicts'>): string | null {
  const parts = [
    row.queuedEdits ? `${plural(row.queuedEdits, 'edit')} waiting to send` : '',
    row.heldEdits ? `${plural(row.heldEdits, 'edit')} held` : '',
    row.conflicts ? plural(row.conflicts, 'conflict') : '',
  ].filter(Boolean);
  return parts.length ? parts.join(', ') : null;
}

/** Why a switch can't start yet, or null. The backend refuses the same. */
export function switchBlocked(row: Pick<SourcesRoleRow, 'queuedEdits' | 'heldEdits' | 'conflicts'> | undefined, unfinished: string | null): string | null {
  if (unfinished) return 'A switch stopped part way. Finish it first.';
  const waiting = row && waitingLine(row);
  return waiting ? `Settle these first: ${waiting}.` : null;
}

/** A switch's dry run as plain lines: counts first, nothing written yet. */
export function planLines(plan: SourcesMigrationPlan): string[] {
  if (plan.refused) return [plan.refused];
  if (plan.nothingToDo) return ['Nothing to change: tasks already live there.'];
  const lines: string[] = [];
  const mirrored = [plan.mirroredTasks ? plural(plan.mirroredTasks, 'task') : '', plan.mirroredProjects ? plural(plan.mirroredProjects, 'project') : ''].filter(Boolean);
  if (mirrored.length) lines.push(`${mirrored.join(' and ')} mirrored now; every file stays in your vault.`);
  lines.push(plan.files.length ? `${plural(plan.files.length, 'file')} in your vault change, each backed up first.` : 'No vault file changes.');
  if (plan.configChange) lines.push(plan.configChange);
  lines.push(plan.trackerWrites ? `${plural(plan.trackerWrites, 'tracker write')}.` : 'Nothing is sent to the tracker, archived or deleted there.');
  lines.push('You can roll it back from this page, byte for byte.');
  return [...lines, ...plan.notes];
}

/** What applying a dry run would do, for the confirm button. */
export function switchVerb(to: SourcesOwnerKind, providerName?: string | null): string {
  return to === 'tracker' ? `Switch to Vault plus ${providerName || DEFAULT_TRACKER}` : 'Switch to Vault only';
}

/** The knowledge list with one more connector (no duplicate). */
export function withReference(list: readonly SourcesReference[], connector: string): SourcesReference[] {
  return list.some(r => r.connector === connector) ? [...list] : [...list, { connector, label: null }];
}

/** The knowledge list without one connector. */
export function withoutReference(list: readonly SourcesReference[], connector: string): SourcesReference[] {
  return list.filter(r => r.connector !== connector);
}

/** A workspace row's name. */
export const workspaceName = (row: Pick<SourcesWorkspaceRow, 'id' | 'title'>) => row.title || row.id;

/** The label a list item shows: its own when the backend sent one, else
 *  the role's (a local task with no label of its own is stored in Hollow). */
export function itemLabel(own: SourceLabel | null | undefined, role: SourceLabel | null | undefined): SourceLabel | null {
  return own ?? role ?? null;
}
