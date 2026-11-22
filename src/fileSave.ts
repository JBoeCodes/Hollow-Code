/**
 * One compare-and-swap save of a workspace file's draft (next-level P08).
 *
 * The save writes a snapshot: the text at the moment Save was pressed. The
 * user can keep typing while it is in flight, so when it lands the editor's
 * text may be newer than what was written. That newer text must stay an
 * unsaved draft against the version just written, and be what the next save
 * writes; it is never replaced by the snapshot (review A, slice 4 item 1d:
 * `WorkFileEditor` used to set its text back to the snapshot and clear the
 * stored draft, dropping those keystrokes). Pure apart from the callbacks, and
 * unit-tested with a save the test lands by hand.
 */

/** What the save call returns (`SaveNoteResult`, narrowed to what is read). */
export interface CasResult { conflict: boolean; note: { content: string; revision?: string } }

/** How a conflict's revision starts when the file on disk can't be read or
 *  isn't text (`vault::UNREADABLE_REVISION`, concurrency PR 2 delta review 3):
 *  its `content` is empty because nothing could be read, not because the
 *  file is empty. */
export const UNREADABLE_REVISION = 'unreadable:';

/** The read error of an unreadable conflict's note, or null for a real one. */
export function unreadableReason(note: { content: string; revision?: string }): string | null {
  const rev = note.revision ?? '';
  return rev.startsWith(UNREADABLE_REVISION) ? rev.slice(UNREADABLE_REVISION.length).trim() : null;
}

/** What a reread of the file decides after an unreadable conflict: still
 *  unreadable, a real conflict (now the disk copy can be compared, adopted
 *  or merged), or back to the draft when the file is what it was based on. */
export type RereadState =
  | { kind: 'unreadable'; reason: string }
  | { kind: 'conflict'; base: string; disk: string; draft: string }
  | { kind: 'draft'; baseline: string; text: string };

export function afterReread(read: { ok: true; content: string } | { ok: false; error: string }, base: string, draft: string): RereadState {
  if (!read.ok) return { kind: 'unreadable', reason: read.error };
  if (read.content === base) return { kind: 'draft', baseline: base, text: draft };
  return { kind: 'conflict', base, disk: read.content, draft };
}

export type SaveOutcome =
  /** Written. `baseline` is what is on disk now; `text` is the editor's text,
   *  which differs from it when typing arrived during the save. */
  | { kind: 'saved'; baseline: string; text: string }
  /** The file changed on disk; nothing was written. `draft` is the text to
   *  offer as "your draft" (the newest, typing in flight included). */
  | { kind: 'conflict'; disk: string; draft: string }
  /** The file changed on disk and can't be read now; nothing was written and
   *  `draft` is kept as a draft against the save's base. There is no disk
   *  text to compare, adopt or merge until a reread succeeds. */
  | { kind: 'unreadable'; reason: string; draft: string };

export interface SaveDeps {
  cas: (path: string, value: string, base: string) => Promise<CasResult>;
  /** The newest text for this file once the save lands: the editor's text,
   *  or its in-memory draft if the editor has moved to another file. */
  latest: () => string | undefined;
  /** Keep `text` as an unsaved draft against `baseline`. */
  keep: (text: string, baseline: string) => void;
  /** Drop the stored draft: everything typed has been written. */
  forget: () => void;
}

/**
 * Save `snapshot` against `base`. With `keepTyped`, text typed after the
 * snapshot survives the save as a draft. A conflict choice passes
 * `keepTyped: false`: the chosen text is exactly what is saved, and the old
 * draft it replaces is not newer typing.
 */
export async function saveSnapshot(path: string, snapshot: string, base: string, deps: SaveDeps, opts: { keepTyped: boolean }): Promise<SaveOutcome> {
  const result = await deps.cas(path, snapshot, base);
  const typed = opts.keepTyped ? deps.latest() : undefined;
  if (result.conflict) {
    const draft = typed ?? snapshot;
    const reason = unreadableReason(result.note);
    if (reason !== null) {
      // Keep exactly what was being saved (a conflict choice's merge too).
      deps.keep(draft, base);
      return { kind: 'unreadable', reason, draft };
    }
    return { kind: 'conflict', disk: result.note.content, draft };
  }
  if (typed !== undefined && typed !== snapshot) {
    deps.keep(typed, snapshot);
    return { kind: 'saved', baseline: snapshot, text: typed };
  }
  deps.forget();
  return { kind: 'saved', baseline: snapshot, text: snapshot };
}

/** One "Read the file again" at a time (delta review 4, 2026-10-20). `start`
 *  hands out a token for the unreadable state it reads for, or null while a
 *  read is pending; `finish` settles the token once and says whether its
 *  response still applies: the state it was for must be the current one (by
 *  identity), so a late response never restores an older draft. */
export function rereadGate<S extends object>() {
  let open: { state: S } | null = null;
  return {
    pending: () => open !== null,
    start(state: S): { state: S } | null {
      if (open) return null;
      open = { state };
      return open;
    },
    finish(token: { state: S }, current: S | null): boolean {
      if (open !== token) return false;
      open = null;
      return token.state === current;
    },
  };
}
