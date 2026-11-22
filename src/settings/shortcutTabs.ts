// Keyboard shortcuts' two tabs (Settings PR 5, plan 3.2): Commands (the keys
// with a chord) and Go to keys (press G, then a key). Pure, so the order and
// labels are tested without the panel.
export type ShortcutTab = 'commands' | 'go';

export const SHORTCUT_TABS: readonly { id: ShortcutTab; label: string }[] = [
  { id: 'commands', label: 'Commands' },
  { id: 'go', label: 'Go to keys' },
];

