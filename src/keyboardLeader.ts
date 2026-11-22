// The go-to leader (five asks A4): press `g` when you aren't typing, then
// one key, to jump anywhere (`g t` Tasks, `g m` Main agent…). A which-key
// card shows the keys while the leader waits. The second keys are
// rebindable (Settings, Keyboard shortcuts, Go to); `g` itself is fixed.
// Pure: the store (`store/leader.ts`) runs it, `goTo.ts` does the jumps.

export const LEADER = 'g';
/** When the card shows; then the leader waits for its second key, Esc or a
 *  click, however long that takes (P1-49). */
export const WHICH_KEY_DELAY_MS = 350;

export interface GoTarget { id: string; label: string; key: string }

/** Every go-to target with its default key, in the card's order. */
export const GO_TARGETS: readonly GoTarget[] = [
  { id: 'home', label: 'Projects', key: 'h' },
  { id: 'main-agent', label: 'COO', key: 'm' },
  // Work's Ask field (One Ask, 2026-11-03): the id Ask your vault had.
  { id: 'ask-vault', label: 'Ask anything', key: 'v' },
  // What's new (n), Runbooks (r) and the workspace map (w) were cut
  // (overnight 11-08 S4). A saved binding for one is dropped on load.
  { id: 'tasks', label: 'Tasks', key: 't' },
  { id: 'library', label: 'Library', key: 'l' },
  { id: 'automations', label: 'Automations', key: 'a' },
  { id: 'mission-control', label: 'Running', key: 'o' },
  { id: 'canvas-gallery', label: 'Canvases', key: 'x' },
  { id: 'waiting', label: 'Needs you', key: 'b' },
  { id: 'history', label: 'Conversation history', key: 'p' },
  { id: 'usage', label: 'Usage', key: 'u' },
  { id: 'settings', label: 'Settings', key: 's' },
  { id: 'quick-chat', label: 'New chat', key: 'q' },
  { id: 'jump', label: 'Jump to a workspace or chat', key: 'j' },
];
export const GO_TARGET_BY_ID: Readonly<Record<string, GoTarget>> = Object.fromEntries(GO_TARGETS.map(t => [t.id, t]));

/** The pages an agent opens with `ui.navigate` (P0-06): the go-to targets
 *  but the two that aren't places (Jump opens a list, New quick chat starts
 *  one). The Rust tool's list (app_actions.rs `ui.navigate`) is this one
 *  plus workspace, session and browser; navigation.test.ts holds them equal. */
export const NAVIGATE_PAGES: readonly string[] = GO_TARGETS.map(t => t.id).filter(id => id !== 'jump' && id !== 'quick-chat');

/** A key a target may have: one lowercase letter or digit. */
export function leaderKeyOk(key: string): boolean {
  return /^[a-z0-9]$/.test(key);
}

/** Target id → key in effect: the default unless changed (`''` = none). */
export function leaderKeyOf(id: string, changed: Readonly<Record<string, string>> = {}): string {
  return id in changed ? changed[id] : GO_TARGET_BY_ID[id]?.key ?? '';
}

/** Key → target id, for the keys in effect. */
export function leaderMap(changed: Readonly<Record<string, string>> = {}): Map<string, string> {
  const map = new Map<string, string>();
  for (const t of GO_TARGETS) {
    const k = leaderKeyOf(t.id, changed);
    if (k && !map.has(k)) map.set(k, t.id);
  }
  return map;
}

/** Give target `id` key `key` (`''` clears it), taking it from whichever
 *  target had it, which then has none. Returns the changes to save (only
 *  what differs from the defaults) and who lost the key. */
export function assignLeader(changed: Readonly<Record<string, string>>, id: string, key: string): { next: Record<string, string>; displaced: string | null } {
  const next: Record<string, string> = { ...changed };
  let displaced: string | null = null;
  if (key) {
    for (const t of GO_TARGETS) {
      if (t.id !== id && leaderKeyOf(t.id, next) === key) { next[t.id] = ''; displaced = t.id; }
    }
  }
  next[id] = key;
  for (const t of GO_TARGETS) if (next[t.id] === t.key) delete next[t.id];
  return { next, displaced };
}

export type LeaderStep =
  | { kind: 'ignore' }
  | { kind: 'start' }
  /** Escape: the leader ends and keeps the key. */
  | { kind: 'cancel' }
  /** The leader ends and the key goes on as if it never waited (a ⌘ chord,
   *  a field or a dialog that took over meanwhile, a dead key). */
  | { kind: 'pass' }
  | { kind: 'run'; target: string }
  | { kind: 'unknown'; key: string };

export interface LeaderContext {
  pending: boolean;
  enabled: boolean;
  typing: boolean;
  modal: boolean;
  recording: boolean;
  keys: Map<string, string>;
}

export interface LeaderKeyEvent { key: string; code?: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey?: boolean; repeat: boolean; isComposing?: boolean }

/** The plain letter or digit a keystroke names, lowercase: `key` for ASCII
 *  (Caps Lock's capital counts, Shift's doesn't), else the physical key
 *  (`code`), so a Cyrillic or Greek layout reaches the same targets. */
export function plainKey(e: LeaderKeyEvent): string | null {
  if (/^[a-z0-9]$/.test(e.key)) return e.key;
  if (/^[A-Z]$/.test(e.key)) return e.shiftKey ? null : e.key.toLowerCase();
  if (e.key.length === 1 && !/^[\x00-\x7f]$/.test(e.key)) {
    const code = e.code ?? '';
    if (/^Key[A-Z]$/.test(code)) return e.shiftKey ? null : code.slice(3).toLowerCase();
    if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  }
  return null;
}

/** What one keydown does to the leader. Waiting: Escape cancels; a bare
 *  modifier is ignored; a ⌘/⌃/⌥ chord, a dead or IME key, or a field, dialog
 *  or recorder that took over meanwhile passes through (ending the leader);
 *  a key in effect runs its target; anything else is unknown. Not waiting:
 *  only a plain `g` outside a text field, a modal and Settings' key recorder
 *  starts it, with shortcuts on. */
export function leaderStep(e: LeaderKeyEvent, cx: LeaderContext): LeaderStep {
  if (cx.pending) {
    if (['Shift', 'Meta', 'Control', 'Alt', 'CapsLock', 'Fn'].includes(e.key)) return { kind: 'ignore' };
    if (e.key === 'Escape') return { kind: 'cancel' };
    if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing || e.key === 'Dead' || e.key === 'Process') return { kind: 'pass' };
    if (cx.typing || cx.modal || cx.recording) return { kind: 'pass' };
    const k = plainKey(e);
    const target = k ? cx.keys.get(k) : undefined;
    return target ? { kind: 'run', target } : { kind: 'unknown', key: e.key };
  }
  if (e.metaKey || e.ctrlKey || e.altKey || e.repeat || e.isComposing) return { kind: 'ignore' };
  if (plainKey(e) !== LEADER) return { kind: 'ignore' };
  if (!cx.enabled || cx.typing || cx.modal || cx.recording) return { kind: 'ignore' };
  return { kind: 'start' };
}

export const TYPING_SELECTOR = '[contenteditable="true"], .cm-editor, [role="textbox"], [role="combobox"], [role="listbox"], [role="menu"], [role="menuitem"]';

/** Whether keystrokes on `t` are typing (a field, a composer, an editor, a
 *  list with type-ahead). */
export function isTypingTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el || typeof el.tagName !== 'string') return false;
  if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT' || el.isContentEditable) return true;
  // Editors, and the custom Select and menus, whose letters are type-ahead.
  return !!el.closest?.(TYPING_SELECTOR);
}
