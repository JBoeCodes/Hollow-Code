// Push-to-talk (overnight 10-11 B7), pure: where dictated words go in a text
// field, and the status words. The store (`store/dictation.ts`) talks to
// `src-tauri/src/dictation.rs` and calls these; unit-tested in
// dictation.test.ts.
//
// The recogniser sends the whole transcript so far each time, so the field
// keeps an anchor: the text before and after the spot dictation started (a
// selection is replaced). Each update rewrites only the dictated middle. If
// the person edits the field meanwhile (the text around the middle no longer
// matches), dictation stops writing rather than overwrite their edit.

export interface DictationStatus { state: 'ready' | 'needsPermission' | 'denied' | 'unavailable'; locale: string | null; reason: string | null }
export interface DictationEvent { session: number; kind: 'partial' | 'final' | 'error' | 'ended'; text: string }

export interface Anchor { before: string; after: string; inserted: string }

/** The anchor for a field whose value is `value` with the selection
 *  `start`–`end` (the selection is replaced by what is said). */
export function beginAnchor(value: string, start: number, end: number): Anchor {
  const a = Math.max(0, Math.min(start, value.length));
  const b = Math.max(a, Math.min(end, value.length));
  // The selection counts as the dictated part until the first words replace it.
  return { before: value.slice(0, a), after: value.slice(b), inserted: value.slice(a, b) };
}

/** The middle to write for `transcript`: a space before it when it would
 *  run into a word, and one after when the text after starts with one. */
export function middleFor(anchor: Anchor, transcript: string): string {
  const t = transcript.trim();
  if (!t) return '';
  const lead = anchor.before && !/\s$/.test(anchor.before) ? ' ' : '';
  const trail = anchor.after && !/^[\s.,;:!?)]/.test(anchor.after) ? ' ' : '';
  return lead + t + trail;
}

/** The field's next value and caret for `transcript`, or null when the
 *  person changed the text around the dictated part (leave it alone). */
export function applyTranscript(anchor: Anchor, current: string, transcript: string): { value: string; caret: number; anchor: Anchor } | null {
  if (current !== anchor.before + anchor.inserted + anchor.after) return null;
  const inserted = middleFor(anchor, transcript);
  const value = anchor.before + inserted + anchor.after;
  // The caret sits after the words, before a trailing space added for them.
  const caret = anchor.before.length + inserted.replace(/ $/, '').length;
  return { value, caret, anchor: { ...anchor, inserted } };
}

/** Whether a keyup ends a held dictation: the shortcut's own key or one of
 *  its modifiers (WebKit sends no keyup for a letter released while ⌘ is
 *  down), released after `HOLD_MS` from the press. A quick tap leaves it
 *  listening (a second tap stops it). */
export const HOLD_MS = 350;
export function endsHold(e: { key: string; code?: string }, chordKey: string, pressedAt: number | null, now: number): boolean {
  if (pressedAt == null || now - pressedAt < HOLD_MS || !chordKey) return false;
  if (e.key === 'Meta' || e.key === 'Shift' || e.key === 'Control' || e.key === 'Alt') return true;
  const k = chordKey.length === 1 ? chordKey.toLowerCase() : chordKey;
  if (e.key.toLowerCase() === k) return true;
  return /^[a-z]$/.test(k) && e.code === `Key${k.toUpperCase()}`;
}

/** A shortcut press this soon after the last one is the key repeating
 *  while held (macOS's first repeat comes ~225 ms in), not a second tap. */
export const REPEAT_MS = 300;
export function isRepeat(lastAt: number | null, now: number): boolean {
  return lastAt != null && now - lastAt < REPEAT_MS;
}

/** The last key of a chord (`CmdOrCtrl+Shift+D` → `D`). */
export function lastKeyOf(chord: string): string {
  const parts = chord.split('+');
  return parts[parts.length - 1] ?? '';
}

/** The mic button's tooltip for a status. */
export function statusTitle(s: DictationStatus | null, shortcut: string): string {
  const keys = shortcut ? ` (${shortcut}: tap, or hold and release)` : '';
  if (!s) return `Dictate${keys}`;
  switch (s.state) {
    case 'ready': return `Dictate on this Mac${s.locale ? `, ${s.locale}` : ''}${keys}`;
    case 'needsPermission': return `Dictate${keys}. macOS asks for speech recognition and the microphone the first time.`;
    default: return s.reason ?? 'Dictation is unavailable.';
  }
}
