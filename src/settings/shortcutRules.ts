import { COMMAND_BY_ID, LOCAL_ALT_CHORDS, displayChord, type Chord } from '../shortcuts';
import { localUsesOf } from '../localKeys';

// Rules Settings' recorder checks beyond macOS's own chords (P1-49);
// shortcuts.rs `validate` refuses the same, so an agent can't set them either.

/** Why a system-wide command can't have `chord`, or null: it takes the
 *  chord from every app, so it needs all of ⌘⌥⇧. */
export function systemWideProblem(id: string, chord: Chord): string | null {
  if (!chord || !COMMAND_BY_ID[id]?.global || chord.startsWith('CmdOrCtrl+Alt+Shift+')) return null;
  return 'A system-wide key works in every app, so it needs ⌥⇧⌘ and a key.';
}

/** Why no command can have `chord`, or null: an in-surface ⌥ key uses it
 *  (⌥⌘T makes a task from selected text), and the window leaves those
 *  chords to their surface, so the command would never run. */
export function localAltProblem(chord: Chord): string | null {
  if (!LOCAL_ALT_CHORDS.includes(chord)) return null;
  const local = localUsesOf(chord)[0];
  return `${displayChord(chord)} is ${local ? `${local.label.toLowerCase()} (${local.surface.toLowerCase()})` : 'an in-surface key'}. Pick another key.`;
}
