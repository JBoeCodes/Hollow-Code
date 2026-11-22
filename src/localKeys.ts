/**
 * In-surface keyboard commands and their policy (next-level P18; the policy
 * is written in docs/reference/keyboard-shortcuts.md, "Local-command keyboard
 * policy", and shown in Settings, Keyboard shortcuts).
 *
 * App shortcuts (`shortcuts.ts` COMMANDS) cover the menu bar and the
 * system-wide keys, and the master switch turns all of them off. The keys
 * below belong to one surface: they work only while that surface is in front
 * (the conversation on screen, the browser panel showing, text selected
 * in a file), never register globally and cannot shadow
 * another app, so they stay on when app shortcuts are off. None of them uses
 * a default app or global chord unless it means that command there (`sameAs`;
 * a unit test checks), and the standard Edit
 * keys (⌘Z, ⌘C, ⌘V, …) keep their Edit meaning inside a surface. Pure data.
 */
import type { Chord } from './shortcuts';

export interface LocalCommand {
  surface: string;
  label: string;
  chord: Chord;
  /** The app command whose key this shares because inside the surface it
   *  means the same thing there (the browser panel's Back is Hollow's Back
   *  for the page, P1-47). Only such a command may share an app key. */
  sameAs?: string;
}

export const LOCAL_COMMANDS: readonly LocalCommand[] = [
  { surface: 'Selected text in a file or note', label: 'Make a task from the selection', chord: 'CmdOrCtrl+Alt+T' },
  { surface: 'Selected text in a file or note', label: 'Ask about the selection', chord: 'CmdOrCtrl+Alt+A' },
  { surface: 'File editor', label: 'Save', chord: 'CmdOrCtrl+S' },
  { surface: 'Conversation', label: 'Find in the conversation', chord: 'CmdOrCtrl+F' },
  { surface: 'Browser panel', label: 'Go to the address bar', chord: 'CmdOrCtrl+L' },
  // The page's own Back, Forward and Reload (BrowserPane.tsx); the menu's
  // Back and Forward step the page too while it has the keys (appMenu.ts).
  { surface: 'Browser panel', label: 'Back', chord: 'CmdOrCtrl+[', sameAs: 'go-back' },
  { surface: 'Browser panel', label: 'Forward', chord: 'CmdOrCtrl+]', sameAs: 'go-forward' },
  { surface: 'Browser panel', label: 'Reload', chord: 'CmdOrCtrl+R' },
];

/** In-surface commands that use `chord`, so recording an app shortcut on it
 *  can say what else answers to it; one that means command `id` there
 *  doesn't count. */
export function localUsesOf(chord: Chord, id?: string): LocalCommand[] {
  return chord ? LOCAL_COMMANDS.filter(c => c.chord === chord && (!id || c.sameAs !== id)) : [];
}
