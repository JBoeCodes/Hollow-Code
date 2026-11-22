// The keyboard shortcuts card's content (overnight F4), built from
// registries rather than written in the component: app-wide rows from
// `COMMANDS` (shortcuts.ts, the keys Settings changes), in-surface chords from
// `LOCAL_COMMANDS` (localKeys.ts), and the unmodified page keys (list and
// suggestion letters, arrows, the chat's send keys) from `pageKeys` below.
// Pure, so the unit tests check every row has a label.

/** One page-local key: which card section, what it does, its keys. `letter`
 *  rows are single-letter shortcuts that go off with "Use keyboard
 *  shortcuts"; arrows, Enter and the chat's send keys always work. */
export interface PageKey {
  section: 'Lists' | 'Suggestions (panel focused)' | 'Notification history' | 'Chat' | 'Ask'
    | 'Annotate' | 'New project' | 'Editing a note' | 'Running';
  label: string;
  keys: readonly string[];
  /** Keys in `keys` that are letters and so go off with shortcuts. */
  letters?: readonly string[];
}

/** `MOD` comes from shortcuts.ts at render time; `palette` is the command
 *  bar's key in effect (`''` when cleared or shortcuts are off). */
export function pageKeys(mod: string, palette = `${mod}K`): readonly PageKey[] {
  return [
    { section: 'Annotate', label: 'move the crosshair 8 pixels (Shift: 40)', keys: ['←', '→', '↑', '↓'] },
    { section: 'Annotate', label: 'place a comment or box corner; edit a numbered mark', keys: ['↵'] },
    { section: 'Annotate', label: 'remove the focused numbered mark', keys: ['⌫', 'Delete'] },
    { section: 'Annotate', label: 'cancel a pending box or finish editing; otherwise close', keys: ['Esc'] },
    { section: 'New project', label: 'create without starting a chat', keys: [`${mod}↵`] },
    { section: 'New project', label: 'cancel the idle form', keys: ['Esc'] },
    { section: 'Lists', label: 'walk rows', keys: ['j', 'k', '↑', '↓'], letters: ['j', 'k'] },
    { section: 'Lists', label: 'open row', keys: ['↵'] },
    { section: 'Lists', label: 'Library Results: back to all results from a focused row (outside Canvases)', keys: ['Esc'] },
    { section: 'Lists', label: 'select a task for bulk edit', keys: ['x'], letters: ['x'] },
    // Five asks A4: the rail, Home's workspaces and Automations too.
    { section: 'Lists', label: 'the row’s menu (↑ ↓ walk it, esc closes); r on Rename, e on Edit or Open', keys: ['.', 'r', 'e'], letters: ['.', 'r', 'e'] },
    { section: 'Lists', label: 'a task row’s menu, with shortcuts off too', keys: ['⇧F10'] },
    { section: 'Suggestions (panel focused)', label: 'walk suggestions', keys: ['j', 'k', '↑', '↓'], letters: ['j', 'k'] },
    { section: 'Suggestions (panel focused)', label: 'accept', keys: ['a'], letters: ['a'] },
    { section: 'Suggestions (panel focused)', label: 'reject (with Undo)', keys: ['⇧R'], letters: ['⇧R'] },
    { section: 'Suggestions (panel focused)', label: 'snooze until 09:00 (with Undo)', keys: ['z'], letters: ['z'] },
    { section: 'Suggestions (panel focused)', label: 'edit', keys: ['e'], letters: ['e'] },
    { section: 'Notification history', label: 'dismiss: mark it read (its × button deletes it)', keys: ['x'], letters: ['x'] },
    { section: 'Chat', label: 'send', keys: ['↵', `${mod}↵`] },
    { section: 'Chat', label: 'new line', keys: ['⇧↵'] },
    { section: 'Chat', label: 'place a suggested prompt', keys: ['⇥'] },
    { section: 'Chat', label: 'prompt history', keys: ['↑', '↓'] },
    { section: 'Chat', label: 'slash commands', keys: ['/'] },
    // Overnight 10-11 A2: a sentence in the command bar. One Ask (2026-11-03):
    // its row asks the way Your work's Ask field would.
    { section: 'Ask', label: 'type a sentence in the command bar, then ↵ on its Ask row', keys: [palette] },
    // Tonight's views (cross-PR review S2: their keys weren't on the card).
    { section: 'Editing a note', label: 'reach the inline AI bar for the selection', keys: [mod === '⌘' ? '⇧⌘↵' : `${mod}⇧↵`] },
    { section: 'Editing a note', label: 'accept a suggestion', keys: ['⇥', `${mod}↵`] },
    { section: 'Editing a note', label: 'reject it', keys: ['Esc'] },
    { section: 'Editing a note', label: 'insert menu at the start of a line', keys: ['/'] },
    { section: 'Running', label: 'walk the turns', keys: ['j', 'k', '↑', '↓'], letters: ['j', 'k'] },
    { section: 'Running', label: 'open its chat', keys: ['↵'] },
    { section: 'Running', label: 'stop it, add a note', keys: ['s', 'n'], letters: ['s', 'n'] },
    { section: 'Running', label: 'next layout: tracks, orbit or list', keys: ['v'], letters: ['v'] },
  ];
}

export interface KeyRow { label: string; keys: string[] }
export interface KeySection { title: string; rows: KeyRow[] }

/** A command as the card needs it: label, group and its key now ('' = none). */
export interface CommandKey { label: string; group: string; key: string }
/** An in-surface command (`LOCAL_COMMANDS`) with its key shown. */
export interface LocalKey { surface: string; label: string; key: string }

/** The card's sections, in order; empty sections and keyless rows are left
 *  out, and letter keys go when shortcuts are off. */
/** A go-to target (five asks A4) with its key after the leader. */
export interface GoKey { label: string; leader: string; key: string }

export function keymapSections(commands: readonly CommandKey[], page: readonly PageKey[], enabled: boolean, local: readonly LocalKey[] = [], goTo: readonly GoKey[] = []): KeySection[] {
  const fromCommands = (group: string): KeyRow[] => commands
    .filter(c => c.group === group && c.key)
    .map(c => ({ label: c.label.toLowerCase(), keys: [c.key] }));
  const fromPage = (section: PageKey['section']): KeyRow[] => page
    .filter(k => k.section === section)
    .map(k => ({ label: k.label, keys: k.keys.filter(key => key && (enabled || !k.letters?.includes(key))) }))
    .filter(r => r.keys.length > 0);
  return [
    { title: 'Navigate', rows: fromCommands('Navigate') },
    // The go-to leader is letters only: gone with shortcuts off.
    { title: 'Go to (press g, then…)', rows: enabled ? goTo.filter(g => g.key).map(g => ({ label: g.label.toLowerCase(), keys: [`${g.leader} ${g.key}`] })) : [] },
    { title: 'Act', rows: [...fromCommands('Create'), ...fromCommands('View')] },
    { title: 'Lists', rows: fromPage('Lists') },
    { title: 'Suggestions (panel focused)', rows: fromPage('Suggestions (panel focused)') },
    { title: 'Notification history', rows: fromPage('Notification history') },
    ...(['Annotate', 'New project', 'Ask', 'Editing a note', 'Running'] as const)
      .map(title => ({ title, rows: fromPage(title) })),
    // The chat's commands (stop the running turn) come from COMMANDS too.
    { title: 'Chat', rows: [...fromPage('Chat'), ...fromCommands('Chat')] },
    // In-surface keys stay on when app shortcuts are off (localKeys.ts).
    { title: 'In a surface', rows: local.filter(l => l.key).map(l => ({ label: `${l.label.toLowerCase()} (${l.surface.toLowerCase()})`, keys: [l.key] })) },
    { title: 'Anywhere (global)', rows: fromCommands('Anywhere') },
  ].filter(sec => sec.rows.length);
}
