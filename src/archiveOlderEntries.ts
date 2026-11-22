// Archive older entries… (V5a, `workspaces.archive_shared_docs`): the confirm
// dialog's text for a workspace's WORKSPACE.md and PLAN.md. The numbers come
// from Rust's dry run; each doc's hash binds the apply, so what this text
// shows is exactly what moves, word for word, into the archive notes.

/** The doc a dry run or apply is for. */
export type SharedDocName = 'WORKSPACE.md' | 'PLAN.md';

/** `archive_shared_docs`' answer (the Rust `SharedDocArchive`, camelCase). */
export interface SharedDocArchive {
  workspace: string;
  doc: SharedDocName;
  mode: 'dry_run' | 'apply';
  today: string;
  /** SHA-256 of the doc now: the apply's `expectedHash`. */
  hash: string;
  newHash: string | null;
  bytesBefore: number;
  bytesAfter: number;
  archiveBytes: number;
  archivePath: string | null;
  moved: string[];
  movedCount: number;
  lossless: boolean;
  applied: boolean;
  busy: boolean;
  runId: string | null;
  warnings: string[];
}

export const SHARED_DOCS: readonly SharedDocName[] = ['WORKSPACE.md', 'PLAN.md'];

/** Whether the dry run moves anything, and safely. */
export const hasMoves = (r: SharedDocArchive): boolean => r.movedCount > 0 && r.lossless;

const kb = (n: number) => n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`;
const noun = (doc: SharedDocName, n: number) => doc === 'PLAN.md' ? `${n} finished section${n === 1 ? '' : 's'}` : `${n} older entr${n === 1 ? 'y' : 'ies'}`;
const SHOWN = 6;

/** The confirm's one-line description, for a screen reader. */
export function archiveSummary(dry: readonly SharedDocArchive[]): string {
  const parts = dry.filter(hasMoves).map(r => `${r.doc} ${kb(r.bytesBefore)} to ${kb(r.bytesAfter)}`);
  return `Moves older text out of ${parts.join(' and ')} into archive notes, word for word.`;
}

/** The confirm's body: per doc, the sizes and the first lines that move. */
export function archiveBody(dry: readonly SharedDocArchive[]): string {
  const lines = ['Older text moves word for word into notes under outputs/workspace-archive/, and a pointer line stays. WORKSPACE.md keeps its Outcome, context and decisions; PLAN.md keeps open work. History & recovery can put it all back.'];
  for (const r of dry.filter(hasMoves)) {
    lines.push('', `${r.doc}: ${kb(r.bytesBefore)} to ${kb(r.bytesAfter)}, ${noun(r.doc, r.movedCount)} (${kb(r.archiveBytes)}) to ${r.archivePath ?? 'the archive'}`);
    for (const first of r.moved.slice(0, SHOWN)) lines.push(`• ${first}`);
    if (r.moved.length > SHOWN) lines.push(`• and ${r.moved.length - SHOWN} more`);
  }
  for (const r of dry.filter(x => x.movedCount > 0 && !x.lossless)) lines.push('', `${r.doc} is left alone: the move would not reproduce it line for line.`);
  return lines.join('\n');
}

/** The footer line after the apply. */
export function archiveDoneMessage(done: readonly SharedDocArchive[]): string {
  const moved = done.filter(r => r.applied);
  if (!moved.length) return 'Nothing was older than the newest entries; nothing moved.';
  return `Archived ${moved.map(r => `${noun(r.doc, r.movedCount)} of ${r.doc}`).join(' and ')}. Undo it in History & recovery.`;
}
