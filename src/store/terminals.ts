// The terminal drawer's shells, per chat (t3code parity 7.2). A tab of the
// Preview pane renders them (`components/TerminalPanel.tsx`), but the shells
// and their screens live here, so leaving the tab, or the chat, keeps the
// output and the running jobs; only closing a terminal, ending the chat or a
// mode that forbids a shell (the backend's `terminal_gate`) ends one.
import { createSignal, type Accessor } from 'solid-js';
import { listen } from '../nativeEvents';
import { api } from '../api';
import { TermScreen } from '../terminalScreen';
import { errMsg } from '../utils';
import { DRAFT_TAB_OWNER, onTerminalTabClosed } from './preview';

export interface TerminalTab {
  id: string;
  /** "Terminal 1", then the shell's own title once it sets one. */
  label: string;
  screen: TermScreen;
  cwd: string;
  /** Bumped when the screen changed (one per animation frame at most). */
  tick: Accessor<number>;
  exited: Accessor<boolean>;
  /** The exit code, null while running or when the shell was killed. */
  code: Accessor<number | null>;
}

interface Inner extends TerminalTab {
  pending: string;
  scheduled: boolean;
  bump: () => void;
  setExited: (v: boolean) => void;
  setCode: (v: number | null) => void;
  setLabel: (v: string) => void;
}

/** Output a frame has not drawn yet, kept to this much (the newest wins). */
export const PENDING_MAX = 2 * 1024 * 1024;
/** A chat with no key yet (its first message is unsent) owns its terminals under this name. */
export const DRAFT_CHAT = DRAFT_TAB_OWNER;

const [byChat, setByChat] = createSignal<Record<string, TerminalTab[]>>({});
const [activeByChat, setActiveByChat] = createSignal<Record<string, string>>({});
const [refusals, setRefusals] = createSignal<Record<string, string>>({});
const inner = new Map<string, Inner>();
const counters = new Map<string, number>();
let listening: Promise<void> | null = null;

/** The chat's terminals, oldest first. */
export const terminalsOf = (chat: string): TerminalTab[] => byChat()[chat] ?? [];
/** The terminal the chat shows (the first when none was picked). */
export function activeTerminal(chat: string): TerminalTab | null {
  const list = terminalsOf(chat);
  return list.find(t => t.id === activeByChat()[chat]) ?? list[0] ?? null;
}
export const setActiveTerminal = (chat: string, id: string) => setActiveByChat(a => ({ ...a, [chat]: id }));
/** Why the chat's mode forbids a terminal, in words; empty when it does not. */
export const terminalRefusal = (chat: string): string => refusals()[chat] ?? '';
export const liveTerminalCount = (chat: string): number => terminalsOf(chat).filter(t => !t.exited()).length;
/** Every chat with a terminal, for the pane's chat-switch cleanup. */
export const chatsWithTerminals = (): string[] => Object.keys(byChat()).filter(c => terminalsOf(c).length > 0);

const frame = (fn: () => void) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => fn()) : setTimeout(fn, 0));

function flush(t: Inner): void {
  t.scheduled = false;
  if (!t.pending) return;
  const data = t.pending;
  t.pending = '';
  t.screen.write(data);
  if (t.screen.title && t.screen.title !== t.label) t.setLabel(t.screen.title);
  t.bump();
}

/** Draw what arrived now (tests; the page draws one frame later by itself). */
export function flushTerminalOutput(): void { for (const t of inner.values()) flush(t); }

function onOutput(id: string, data: string): void {
  const t = inner.get(id);
  if (!t) return;
  t.pending += data;
  if (t.pending.length > PENDING_MAX) t.pending = t.pending.slice(t.pending.length - PENDING_MAX);
  if (!t.scheduled) { t.scheduled = true; frame(() => flush(t)); }
}

function onExit(id: string, code: number | null): void {
  const t = inner.get(id);
  if (!t) return;
  flush(t);
  t.setExited(true);
  t.setCode(code);
  t.screen.write(`\r\n\x1b[2m[The shell ended${code === null ? '' : ` with code ${code}`}. Close this terminal or open a new one.]\x1b[0m\r\n`);
  t.bump();
}

/** Listen for the backend's output and exit events, once. */
export function listenToTerminals(): Promise<void> {
  if (!listening) {
    listening = Promise.all([
      listen<{ id: string; data: string }>('terminal-output', e => onOutput(e.payload.id, e.payload.data)),
      listen<{ id: string; code: number | null }>('terminal-exit', e => onExit(e.payload.id, e.payload.code)),
    ]).then(() => undefined).catch(() => { /* browser mock: no events */ });
  }
  return listening;
}

/** Feed the store directly (tests and the browser mock). */
export const terminalEvents = { output: onOutput, exit: onExit };

export interface OpenContext { agent: string | null; perm: string; cwd: string }

/** Start a terminal for `chat`, sized `rows` by `cols`. Returns the refusal
 *  or error in words, or null when it opened. */
export async function openTerminal(chat: string, ctx: OpenContext, rows: number, cols: number): Promise<string | null> {
  await listenToTerminals();
  let opened;
  try {
    opened = await api.terminalOpen(chat, ctx.agent, ctx.perm, ctx.cwd, rows, cols);
  } catch (e) {
    const text = errMsg(e);
    setRefusals(r => ({ ...r, [chat]: text }));
    return text;
  }
  setRefusals(r => { const { [chat]: _gone, ...rest } = r; return rest; });
  const n = (counters.get(chat) ?? 0) + 1;
  counters.set(chat, n);
  const [tick, setTick] = createSignal(0);
  const [exited, setExited] = createSignal(false);
  const [code, setCode] = createSignal<number | null>(null);
  const [label, setLabel] = createSignal(`Terminal ${n}`);
  const screen = new TermScreen(rows, cols, { onReply: data => void api.terminalWrite(opened.id, data).catch(() => {}) });
  const tab: Inner = {
    id: opened.id, get label() { return label(); }, screen, cwd: opened.cwd, tick, exited, code,
    pending: '', scheduled: false, bump: () => setTick(screen.version), setExited, setCode, setLabel,
  };
  inner.set(opened.id, tab);
  setByChat(m => ({ ...m, [chat]: [...(m[chat] ?? []), tab] }));
  setActiveTerminal(chat, opened.id);
  return null;
}

/** Type into a terminal (key presses, pastes). */
export function typeInto(id: string, data: string): void {
  if (inner.get(id)?.exited()) return;
  void api.terminalWrite(id, data).catch(() => { /* closed meanwhile */ });
}

/** The tab's screen follows the pane's size. */
export function resizeTerminal(id: string, rows: number, cols: number): void {
  const t = inner.get(id);
  if (!t || (t.screen.rows === rows && t.screen.cols === cols)) return;
  t.screen.resize(rows, cols);
  t.bump();
  void api.terminalResize(id, rows, cols).catch(() => {});
}

function drop(chat: string, id: string): void {
  inner.delete(id);
  setByChat(m => {
    const rest = (m[chat] ?? []).filter(t => t.id !== id);
    if (rest.length) return { ...m, [chat]: rest };
    const { [chat]: _gone, ...others } = m;
    return others;
  });
}

/** Close one terminal: its shell and everything it started end. */
export function closeTerminal(chat: string, id: string): void {
  const list = terminalsOf(chat);
  const at = list.findIndex(t => t.id === id);
  if (at < 0) return;
  const wasActive = activeTerminal(chat)?.id === id;
  drop(chat, id);
  if (wasActive) {
    const next = list[at + 1] ?? list[at - 1];
    if (next) setActiveTerminal(chat, next.id);
  }
  void api.terminalClose(id).catch(() => {});
}

/** Close every terminal of a chat (the drawer's close, or the chat ended). */
export function closeChatTerminals(chat: string): void {
  for (const t of terminalsOf(chat)) closeTerminal(chat, t.id);
}

/** Forget a chat's terminals the backend already ended (a mode that forbids a shell). */
function forgetChat(chat: string): void {
  for (const t of terminalsOf(chat)) drop(chat, t.id);
}

/** Ask the backend whether the chat's mode still allows a shell. When it does
 *  not, its terminals are gone (the backend ended them) and the reason is kept
 *  for the tab to show. Returns the reason, or '' when a terminal is allowed. */
export async function checkTerminalGate(chat: string, agent: string | null, perm: string): Promise<string> {
  let reason: string | null = null;
  let failed = false;
  try { reason = await api.terminalGate(chat, agent, perm); } catch (e) { reason = errMsg(e); failed = true; }
  if (reason) {
    // A refusal means the backend ended them already; a failed call means we
    // cannot know, so close each one (closing is idempotent).
    if (failed) closeChatTerminals(chat); else forgetChat(chat);
    setRefusals(r => ({ ...r, [chat]: reason! }));
    return reason;
  }
  setRefusals(r => { if (!(chat in r)) return r; const { [chat]: _gone, ...rest } = r; return rest; });
  return '';
}

/** A chat got its key: its terminals follow it from the draft name. */
export function adoptDraftTerminals(to: string): void {
  const from = DRAFT_CHAT;
  const list = byChat()[from];
  if (!list?.length || to === from) return;
  setByChat(m => {
    const { [from]: moved, ...rest } = m;
    return { ...rest, [to]: [...(rest[to] ?? []), ...(moved ?? [])] };
  });
  setActiveByChat(a => { const { [from]: pick, ...rest } = a; return pick ? { ...rest, [to]: pick } : rest; });
  const n = counters.get(from);
  if (n) { counters.set(to, Math.max(n, counters.get(to) ?? 0)); counters.delete(from); }
  void api.terminalAdoptChat(from, to).catch(() => {});
}

// A Terminal tab closed by hand ends that chat's shells (the tab's own x, after
// its confirm); a tab dropped because another chat came on screen does not.
onTerminalTabClosed(closeChatTerminals);

/** Tests only: a fresh store (no backend calls). */
export function resetTerminalsForTests(): void {
  inner.clear(); counters.clear(); listening = null;
  setByChat({}); setActiveByChat({}); setRefusals({});
}
