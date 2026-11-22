/**
 * Project scripts' pure half (t3code parity 4.5, A10): the shape the window and
 * `project_scripts.rs` share, and the chord guard a script's chord passes.
 * A script's chord is always ⌘⇧ and one letter or digit (no ⌥, so it never types
 * a character or collides with a window manager), is never a macOS chord
 * (`macReserved.ts`), never one a Hollow command holds (`activeChord`), and never
 * another script's. The rules are code here and test in `projectScripts.test.ts`.
 */
import { COMMANDS, activeChord, chordFromEvent, displayChord, type ChordEvent } from './shortcuts';
import { reservedReason } from './macReserved';

export interface ProjectScript {
  id: string;
  name: string;
  command: string;
  chord?: string | null;
  /** The service a run is (`script-<slug>`); filled by the backend. */
  serviceName?: string;
}

/** The keys a script chord can use, in the order the editor lists them. */
export const SCRIPT_CHORD_KEYS: readonly string[] = [...'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'];
export const MAX_SCRIPTS = 12;
const SHAPE = /^CmdOrCtrl\+Shift\+[A-Z0-9]$/;

export const scriptChord = (key: string) => `CmdOrCtrl+Shift+${key}`;

/** Why `chord` can't be a script's chord, or null. `others` are the project's other scripts. */
export function scriptChordProblem(chord: string, others: readonly Pick<ProjectScript, 'name' | 'chord'>[]): string | null {
  if (!SHAPE.test(chord)) return 'A script chord is ⇧⌘ and one letter or digit.';
  const shown = displayChord(chord);
  const reserved = reservedReason(chord);
  if (reserved) return `${shown} is ${reserved} on a Mac.`;
  const command = COMMANDS.find(c => activeChord(c.id) === chord);
  if (command) return `${shown} is Hollow's ${command.label}.`;
  const script = others.find(s => s.chord === chord);
  if (script) return `${shown} runs ${script.name}.`;
  return null;
}

/** The script a keystroke runs, or null: a ⌘⇧ chord some script holds and the guard still allows
 *  (a Hollow command may have taken the chord since it was saved). */
export function scriptForChord(e: ChordEvent, scripts: readonly ProjectScript[]): ProjectScript | null {
  if (!e.shiftKey || e.altKey) return null;
  const chord = chordFromEvent(e);
  if (!chord) return null;
  const script = scripts.find(s => s.chord === chord);
  if (!script) return null;
  return scriptChordProblem(chord, scripts.filter(s => s !== script && s.chord !== chord)) ? null : script;
}
