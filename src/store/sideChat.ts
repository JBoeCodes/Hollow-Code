// Side chat (t3code parity 7.6, `side_chat.rs`, docs/SIDE_CHAT.md): a second chat beside
// the one on screen, for a quick question about it that must not disturb its run.
//
// This module is NOT chatRuntime.ts. The main chat's runtime is one global conversation
// (transcript, composer, busy flag); a side chat keeps its own small state here and
// reads only the `chat-event`s tagged with its own conversation key, so nothing it does
// reaches the main chat's transcript, busy flag or composer. The main runtime routes
// those same events as `orphan` (another conversation's) and ignores them.
//
// What goes back to the main chat is only what the person sends with the pane's visible
// button: `addToMainChat` puts words in the main chat's message box as a draft. Nothing is
// sent and nothing is written to the main transcript until they press Send there.
import { createSignal } from 'solid-js';
import { api } from '../api';
import { listen } from '../nativeEvents';
import type { AgentConversationTurn, AgentSessionSummary } from '../types';
import { errMsg } from '../utils';
import { newClientTurnId } from '../chatQueue';

const OPEN_KEY = 'hollow.sideChatOpen';
const ACCESS_KEY = 'hollow.sideChatSameAccess';

function read(key: string): string | null { try { return localStorage.getItem(key); } catch { return null; } }
function write(key: string, value: string) { try { localStorage.setItem(key, value); } catch { /* the session value still works */ } }

/** Whether the pane is open. Remembered, like the other panes. */
export const [sideChatOpen, setSideChatOpen] = createSignal(read(OPEN_KEY) === 'true');
export function openSideChat() { setSideChatOpen(true); write(OPEN_KEY, 'true'); }
export function closeSideChat() { setSideChatOpen(false); write(OPEN_KEY, 'false'); }
export function toggleSideChat() { if (sideChatOpen()) closeSideChat(); else openSideChat(); }

/** The visible button, the conversation menu item and the palette row. A chat that can't have a
 *  side chat yet (no first message) still opens the pane, which says why in its own words. */
export const toggleSideChatChecked = toggleSideChat;

/** Whether the side chat may use the main chat's own access (never more than it has).
 *  Off by default: a side chat reads and answers, and changes nothing. */
export const [sideChatSameAccess, setSideChatSameAccessSignal] = createSignal(read(ACCESS_KEY) === 'true');
export function setSideChatSameAccess(on: boolean) { setSideChatSameAccessSignal(on); write(ACCESS_KEY, String(on)); }

/** A chat can have a side chat once it has a key (its first turn started). The Main agent
 *  never does: its page stays its own (the button is not offered there, and the backend refuses). */
export function sideChatBlockedReason(parentKey: string | null): string | null {
  return parentKey ? null : 'Send a first message in this chat first: a side chat reads what the chat has said.';
}

export interface SideChatState {
  /** The main chat this side chat sits beside. */
  parent: string | null;
  /** The side chat's own conversation key (none until opened). */
  key: string | null;
  turns: AgentConversationTurn[];
  /** The question just sent, before the stored turns include it. */
  pending: string | null;
  /** The reply as it streams: streamed pieces, else whole blocks. */
  live: { text: string; tools: string[] } | null;
  busy: boolean;
  loading: boolean;
  /** Why it can't open or send, or the turn's error. */
  error: string | null;
  /** The agent asked a question card: it is answered from Waiting on you. */
  asked: boolean;
  /** The parent's earlier side chats (the picker). */
  earlier: AgentSessionSummary[];
}

const empty = (parent: string | null = null): SideChatState => ({ parent, key: null, turns: [], pending: null, live: null, busy: false, loading: false, error: null, asked: false, earlier: [] });
export const [sideChat, setSideChat] = createSignal<SideChatState>(empty());

let streamed = '';
let blocks = '';
let bindToken = 0;

/** Read the stored turns of side chat `key` (none while it is new). */
async function readTurns(key: string): Promise<AgentConversationTurn[]> {
  const conversation = await api.readAgentConversation(key);
  return conversation.turns.filter(t => t.role === 'user' || t.role === 'assistant');
}

/** Show the side chat of `parent` (its newest, or a new one when `fresh`). The main chat is
 *  only read. Safe to call again for the same parent: a later call for another parent wins. */
export async function bindSideChat(parent: string | null, fresh = false): Promise<void> {
  const mine = ++bindToken;
  streamed = ''; blocks = '';
  setSideChat(empty(parent));
  const blocked = sideChatBlockedReason(parent);
  if (blocked || !parent) { setSideChat(s => ({ ...s, error: blocked })); return; }
  setSideChat(s => ({ ...s, loading: true }));
  try {
    const opened = await api.sideChatOpen(parent, fresh);
    if (mine !== bindToken) return;
    const [turns, earlier] = await Promise.all([
      opened.turnCount > 0 ? readTurns(opened.key) : Promise.resolve([] as AgentConversationTurn[]),
      api.sideChatList(parent).catch(() => [] as AgentSessionSummary[]),
    ]);
    if (mine !== bindToken) return;
    setSideChat(s => ({ ...s, key: opened.key, turns, earlier, loading: false }));
  } catch (e) {
    if (mine !== bindToken) return;
    setSideChat(s => ({ ...s, loading: false, error: errMsg(e) }));
  }
}

/** Open one of the parent's earlier side chats. */
export async function openEarlierSideChat(key: string): Promise<void> {
  const s = sideChat();
  if (!s.parent || key === s.key || s.busy) return;
  const mine = ++bindToken;
  streamed = ''; blocks = '';
  setSideChat({ ...s, key, turns: [], pending: null, live: null, error: null, asked: false, loading: true });
  try {
    const turns = await readTurns(key);
    if (mine !== bindToken) return;
    setSideChat(cur => ({ ...cur, turns, loading: false }));
  } catch (e) {
    if (mine !== bindToken) return;
    setSideChat(cur => ({ ...cur, loading: false, error: errMsg(e) }));
  }
}

/** Start a new, empty side chat for the same main chat. */
export async function newSideChat(): Promise<void> {
  const s = sideChat();
  if (s.parent && !s.busy) await bindSideChat(s.parent, true);
}

/** Ask the side chat. False (the words stay in the box) when nothing was sent. */
export async function sendSideChat(text: string): Promise<boolean> {
  const words = text.trim();
  const s = sideChat();
  if (!words || !s.key || s.busy || s.loading) return false;
  const key = s.key;
  streamed = ''; blocks = '';
  setSideChat(cur => ({ ...cur, pending: words, error: null, asked: false, busy: true, live: { text: '', tools: [] } }));
  try {
    await api.sideChatSend(key, words, newClientTurnId(), sideChatSameAccess());
    return true;
  } catch (e) {
    setSideChat(cur => cur.key === key ? { ...cur, pending: null, busy: false, live: null, error: errMsg(e) } : cur);
    return false;
  }
}

/** Stop the side chat's running turn. The main chat's turn is never touched: the call
 *  names the side chat's own key. */
export async function stopSideChat(): Promise<void> {
  const key = sideChat().key;
  if (!key) return;
  try { await api.chatCancel(key); } catch (e) { setSideChat(cur => ({ ...cur, error: errMsg(e) })); }
}

/** Fold a settled turn back into the stored transcript. */
async function settle(key: string): Promise<void> {
  try {
    const turns = await readTurns(key);
    setSideChat(cur => cur.key === key ? { ...cur, turns, pending: null, live: null, busy: false } : cur);
    const parent = sideChat().parent;
    if (parent) void api.sideChatList(parent).then(earlier => setSideChat(cur => cur.parent === parent ? { ...cur, earlier } : cur), () => {});
  } catch (e) {
    setSideChat(cur => cur.key === key ? { ...cur, pending: null, live: null, busy: false, error: errMsg(e) } : cur);
  }
}

/** One `chat-event`, only when it names this side chat's own conversation. */
export function applySideEvent(p: { type: string; conversationKey?: string | null; text?: string; name?: string; message?: string }): void {
  const key = sideChat().key;
  if (!key || p.conversationKey !== key) return;
  switch (p.type) {
    case 'runStarted':
      setSideChat(cur => ({ ...cur, busy: true, live: cur.live ?? { text: '', tools: [] } }));
      break;
    case 'assistantDelta':
      streamed += p.text ?? '';
      setSideChat(cur => ({ ...cur, busy: true, live: { text: streamed || blocks, tools: cur.live?.tools ?? [] } }));
      break;
    case 'assistantText':
      blocks += (blocks ? '\n\n' : '') + (p.text ?? '');
      setSideChat(cur => ({ ...cur, busy: true, live: { text: streamed || blocks, tools: cur.live?.tools ?? [] } }));
      break;
    case 'toolUse':
      if (p.name) setSideChat(cur => ({ ...cur, live: { text: cur.live?.text ?? '', tools: [...(cur.live?.tools ?? []), p.name!] } }));
      break;
    case 'interaction':
      setSideChat(cur => ({ ...cur, asked: true }));
      break;
    case 'error':
      setSideChat(cur => ({ ...cur, error: p.message ?? 'The side chat hit an error.' }));
      break;
    case 'done':
    case 'cancelled':
      if (p.type === 'cancelled' && (p as { queued?: boolean }).queued) { setSideChat(cur => ({ ...cur, pending: null, live: null, busy: false })); break; }
      if (p.type === 'done') void settle(key);
      break;
    default:
  }
}

listen<Parameters<typeof applySideEvent>[0]>('chat-event', e => applySideEvent(e.payload)).catch(() => {/* not in tauri */});

/** The words of a reply, ready for the main chat's message box. */
export function resultForMain(text: string): string {
  return `From the side chat:\n\n${text.trim()}`;
}
