// macOS's own shortcuts (five asks A4): chords no Hollow command may take,
// so Hollow never shadows what every Mac app does. Hollow's chords are always
// ⌘ (+⌥/⇧) + one key, so ⌥-letter / ⌥-digit chords (they type characters,
// and window managers take them), ⌃⌥ chords (Magnet's defaults) and ⌃-arrow
// (Mission Control) can't be written at all; the list below covers the rest.
// `macReserved.test.ts` fails on any default binding here, and Settings
// refuses to record one.
import type { Chord } from './shortcuts';

export const MAC_RESERVED: Readonly<Record<Chord, string>> = {
  'CmdOrCtrl+Q': 'Quit', 'CmdOrCtrl+W': 'Close Window', 'CmdOrCtrl+H': 'Hide Hollow', 'CmdOrCtrl+M': 'Minimize',
  'CmdOrCtrl+Tab': 'switching apps', 'CmdOrCtrl+Space': 'Spotlight', 'CmdOrCtrl+`': 'switching windows', 'CmdOrCtrl+,': 'Settings',
  'CmdOrCtrl+C': 'Copy', 'CmdOrCtrl+V': 'Paste', 'CmdOrCtrl+X': 'Cut', 'CmdOrCtrl+Z': 'Undo', 'CmdOrCtrl+Shift+Z': 'Redo',
  'CmdOrCtrl+A': 'Select All', 'CmdOrCtrl+S': 'Save', 'CmdOrCtrl+F': 'Find', 'CmdOrCtrl+G': 'Find Next', 'CmdOrCtrl+Shift+G': 'Find Previous',
  'CmdOrCtrl+P': 'Print', 'CmdOrCtrl+N': 'New', 'CmdOrCtrl+O': 'Open', 'CmdOrCtrl+T': 'New Tab', 'CmdOrCtrl+R': 'Reload',
  'CmdOrCtrl+Alt+H': 'Hide Others', 'CmdOrCtrl+Alt+M': 'Minimize All', 'CmdOrCtrl+Alt+Escape': 'Force Quit', 'CmdOrCtrl+Alt+D': 'Dock hiding',
  'CmdOrCtrl+Shift+3': 'a screenshot', 'CmdOrCtrl+Shift+4': 'a screenshot', 'CmdOrCtrl+Shift+5': 'screenshots and recording',
  'CmdOrCtrl+Shift+/': 'Help', 'CmdOrCtrl+?': 'Help',
  'CmdOrCtrl+Shift+Q': 'Log Out', 'CmdOrCtrl+Alt+Shift+Q': 'Log Out', 'CmdOrCtrl+Alt+Shift+V': 'Paste and Match Style',
  'CmdOrCtrl+Alt+Shift+Escape': 'Force Quit',
  // Accessibility Zoom's keys (on or off, they're macOS's), Cancel, and the Touch Bar screenshot (P1-49).
  'CmdOrCtrl+Alt+8': 'Zoom', 'CmdOrCtrl+Alt+=': 'Zoom In', 'CmdOrCtrl+Alt+-': 'Zoom Out', 'CmdOrCtrl+Alt+\\': 'Zoom smoothing',
  'CmdOrCtrl+.': 'Cancel', 'CmdOrCtrl+Shift+6': 'a Touch Bar screenshot',
  // ⌘-arrows move the caret to a line's or document's ends in every text field.
  'CmdOrCtrl+ArrowLeft': 'start of line', 'CmdOrCtrl+ArrowRight': 'end of line', 'CmdOrCtrl+ArrowUp': 'start of document', 'CmdOrCtrl+ArrowDown': 'end of document',
  ...Object.fromEntries(Array.from({ length: 12 }, (_, i) => [`CmdOrCtrl+F${i + 1}`, 'a function key'])),
};

/** A reserved chord Hollow uses for exactly its macOS meaning: ⌘, opens
 *  Settings and ⌘N makes a new conversation (File → New). Only that command
 *  may hold it. */
export const STANDARD_MEANING: Readonly<Record<Chord, string>> = {
  'CmdOrCtrl+,': 'settings',
  'CmdOrCtrl+N': 'new-conversation',
};

/** Why command `id` can't have `chord` (what macOS uses it for), or null. */
export function reservedReason(chord: Chord, id?: string): string | null {
  // Every F-key, with any modifiers: macOS and the keyboard's own row use them.
  if (/^CmdOrCtrl\+(Alt\+)?(Shift\+)?F([1-9]|1[0-2])$/.test(chord)) return 'a function key';
  const what = MAC_RESERVED[chord];
  if (!what) return null;
  if (id && STANDARD_MEANING[chord] === id) return null;
  return what;
}
