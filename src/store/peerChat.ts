// M2 PR 3: one open remote chat, read live from its home Mac
// (`docs/reference/peers.md`). The turns live here, in the webview's memory,
// only while the view is open; closing it drops them and stops the stream.
//
// Pure parts (what the tests hold):
// - `readTurns` reads the wire's turns defensively (another Mac sent them).
// - `foldPeerChat` applies a `peer-chat-event` as the phone contract's client
//   rules say: `live` replaces the live view (null clears it); `append` adds
//   to it only when its `ord` and `gen` match the view held (a mismatch is
//   dropped: the Rust follower fetches the chat again and a `detail` event
//   follows); `turn` drops the live view (a `detail` follows); `state` sets
//   the row's state; `detail` replaces the newest turns and keeps older pages
//   already loaded.
// - `wireQuestionsToCard` turns the wire's card into the `ChatInteraction`
//   the existing QuestionCard renders (every card arrives as questions:
//   the home flattens confirmation, checkbox and verdicts cards), and
//   `cardAnswerToWire` turns its answer back into the route's `answers` map.
// The answer goes to the chat's home (`peer_answer`); this Mac's own cards
// and Waiting on you never see a remote card.
import { createSignal } from 'solid-js';
import { invoke } from '../api';
import { onNative } from '../nativeEvents';
import type { Accepted, MachineRef, PeerChatDetail, PeerChatEvent, PeerFailure, RemoteLive, RemoteSession } from '../generated/ipc';
import type { ChatInteraction, InteractionAnswer, InteractionQuestion } from '../types';
import { isPeerFailure } from './peerSessions';

export interface WireTool { id: string; name: string; status: string; detail: string | null }
export interface WireQuestionOption { id: string; label: string; description: string | null; recommended: boolean }
export interface WireQuestion {
  id: string;
  header: string;
  prompt: string;
  mode: 'single' | 'multi' | 'text';
  required: boolean;
  allowsOther: boolean;
  options: WireQuestionOption[];
}
export interface WireQuestions { id: string; state: 'pending' | 'closed'; items: WireQuestion[] }
export interface WireAttachment { id: string; name: string; kind: string; mime: string; size: number; available: boolean }
export interface WireTurn {
  id: string;
  ord: number;
  role: 'user' | 'assistant';
  text: string;
  agent: string;
  startedAt: string;
  status: string | null;
  error: string | null;
  truncated: boolean;
  tools: WireTool[];
  questions: WireQuestions | null;
  attachments: WireAttachment[];
}

export interface RemoteChat {
  machineId: string;
  key: string;
  home: MachineRef | null;
  session: RemoteSession | null;
  permissionMode: string | null;
  /** Oldest first. */
  turns: WireTurn[];
  hasMore: boolean;
  live: RemoteLive | null;
  /** The stream is open. */
  connected: boolean;
  /** Why it can't reach the home now (the stream dropped or a call failed). */
  offline: string | null;
  /** The stream stopped for good (pairing needed, an update, the chat gone). */
  ended: PeerFailure | null;
}

export function emptyRemoteChat(machineId: string, key: string): RemoteChat {
  return { machineId, key, home: null, session: null, permissionMode: null, turns: [], hasMore: false, live: null, connected: false, offline: null, ended: null };
}

// ── Reading what another Mac sent ────────────────────────────────────────

const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null && !Array.isArray(v) ? v as Record<string, unknown> : {});
const str = (v: unknown, or = ''): string => (typeof v === 'string' ? v : or);
const strOrNull = (v: unknown): string | null => (typeof v === 'string' ? v : null);
const num = (v: unknown): number => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);

function readTool(v: unknown): WireTool {
  const t = obj(v);
  return { id: str(t.id), name: str(t.name), status: str(t.status, 'done'), detail: strOrNull(t.detail) };
}

function readQuestions(v: unknown): WireQuestions | null {
  if (v == null) return null;
  const q = obj(v);
  const id = str(q.id);
  if (!id) return null;
  const items: WireQuestion[] = list(q.items).map(raw => {
    const i = obj(raw);
    const mode: WireQuestion['mode'] = i.mode === 'multi' || i.mode === 'text' ? i.mode : 'single';
    return {
      id: str(i.id),
      header: str(i.header),
      prompt: str(i.prompt),
      mode,
      required: i.required === true,
      allowsOther: i.allowsOther === true,
      options: list(i.options).map(o => {
        const p = obj(o);
        return { id: str(p.id), label: str(p.label), description: strOrNull(p.description), recommended: p.recommended === true };
      }),
    };
  }).filter(i => i.id);
  return { id, state: q.state === 'pending' ? 'pending' : 'closed', items };
}

/** The wire's turns, read leniently; a turn without an id or ord is skipped. */
export function readTurns(raw: readonly unknown[]): WireTurn[] {
  const out: WireTurn[] = [];
  for (const v of raw) {
    const t = obj(v);
    const id = str(t.id);
    const ord = num(t.ord);
    if (!id || ord < 1) continue;
    out.push({
      id,
      ord,
      role: t.role === 'user' ? 'user' : 'assistant',
      text: str(t.text),
      agent: str(t.agent),
      startedAt: str(t.startedAt),
      status: strOrNull(t.status),
      error: strOrNull(t.error),
      truncated: t.truncated === true,
      tools: list(t.tools).map(readTool),
      questions: readQuestions(t.questions),
      attachments: list(t.attachments).map(a => {
        const x = obj(a);
        return { id: str(x.id), name: str(x.name), kind: str(x.kind), mime: str(x.mime), size: num(x.size), available: x.available === true };
      }),
    });
  }
  return out;
}

function readLive(v: unknown): RemoteLive | null {
  if (v == null) return null;
  const l = obj(v);
  if (typeof l.ord !== 'number' || typeof l.gen !== 'number') return null;
  return { key: str(l.key), ord: l.ord, gen: l.gen, startedAt: str(l.startedAt), text: str(l.text), truncated: l.truncated === true, tools: list(l.tools) };
}

// ── The live fold ────────────────────────────────────────────────────────

/** `detail`'s newest turns, after the older pages already loaded. */
function withDetail(chat: RemoteChat, d: PeerChatDetail): RemoteChat {
  const fresh = readTurns(d.detail.turns);
  const first = fresh.length ? fresh[0].ord : Infinity;
  const older = chat.turns.filter(t => t.ord < first);
  return {
    ...chat,
    home: d.home,
    session: d.detail.session,
    permissionMode: d.detail.permissionMode,
    turns: [...older, ...fresh],
    hasMore: older.length ? chat.hasMore : d.detail.hasMore,
    live: readLive(d.detail.live),
    offline: null,
  };
}

/** An earlier page (`peer_detail` with `before`) put in front. */
export function withEarlier(chat: RemoteChat, d: PeerChatDetail): RemoteChat {
  const known = new Set(chat.turns.map(t => t.id));
  const earlier = readTurns(d.detail.turns).filter(t => !known.has(t.id));
  return { ...chat, turns: [...earlier, ...chat.turns], hasMore: d.detail.hasMore };
}

/** Apply one `peer-chat-event`; events for another chat leave it as it was. */
export function foldPeerChat(chat: RemoteChat, event: PeerChatEvent): RemoteChat {
  if (event.machineId !== chat.machineId || event.key !== chat.key) return chat;
  const data = obj(event.data);
  switch (event.kind) {
    case 'detail':
      return withDetail(chat, event.data as PeerChatDetail);
    case 'live':
      return { ...chat, live: readLive(event.data) };
    case 'append': {
      const live = chat.live;
      if (!live || data.ord !== live.ord || data.gen !== live.gen || typeof data.text !== 'string') return chat;
      return { ...chat, live: { ...live, text: live.text + data.text } };
    }
    case 'turn':
      return { ...chat, live: null };
    case 'state': {
      if (!chat.session || data.key !== chat.key) return chat;
      return { ...chat, session: { ...chat.session, state: str(data.state, chat.session.state), needsAnswer: data.needsAnswer === true } };
    }
    case 'connected':
      return { ...chat, connected: true, offline: null, ended: null };
    case 'disconnected':
      return { ...chat, connected: false, offline: str(data.message) || null };
    case 'ended':
      return { ...chat, connected: false, ended: isPeerFailure(event.data) ? event.data : null };
    default:
      return chat;
  }
}

// ── Cards answered at the chat's home ────────────────────────────────────

/** The chat's open card: the newest assistant turn's pending questions. */
export function openCardOf(turns: readonly WireTurn[]): { turn: WireTurn; questions: WireQuestions } | null {
  for (let i = turns.length - 1; i >= 0; i--) {
    const t = turns[i];
    if (t.role === 'assistant' && t.questions?.state === 'pending') return { turn: t, questions: t.questions };
  }
  return null;
}

/** The wire's card as the QuestionCard's `ChatInteraction` (a closed card is
 *  `stale`: shown, not answerable). */
export function wireQuestionsToCard(key: string, turn: WireTurn, questions: WireQuestions, homeName: string): ChatInteraction {
  const items: InteractionQuestion[] = questions.items.map(q => ({
    id: q.id,
    prompt: q.prompt,
    ...(q.header ? { header: q.header } : {}),
    required: q.required,
    answerMode: q.mode,
    options: q.options.map(o => ({ id: o.id, label: o.label, ...(o.description ? { description: o.description } : {}), ...(o.recommended ? { recommended: true } : {}) })),
    allowCustom: q.allowsOther && q.mode !== 'text',
  }));
  const pending = questions.state === 'pending';
  return {
    id: questions.id,
    sessionKey: key,
    turnOrd: turn.ord,
    runId: '',
    agent: turn.agent,
    nativeSessionId: null,
    toolUseId: '',
    source: 'mcp',
    mode: 'questions',
    request: { mode: 'questions', supersedeOnUserMessage: false, questions: items },
    targetPath: null,
    supersedeOnUserMessage: false,
    status: pending ? 'pending' : 'stale',
    statusReason: pending ? null : `This card was answered or replaced on ${homeName}.`,
    supersededBy: null,
    answer: null,
    createdAt: turn.startedAt,
    resolvedAt: null,
  };
}

/** The QuestionCard's answer as the route's `answers`: `single` an option id
 *  or the person's words, `multi` a list (their words last), `text` the
 *  text; an unanswered item is null. */
export function cardAnswerToWire(questions: WireQuestions, answer: InteractionAnswer): Record<string, string | string[] | null> {
  const out: Record<string, string | string[] | null> = {};
  for (const q of questions.items) {
    const a = answer.answers?.[q.id];
    const other = a?.other?.trim() || null;
    if (q.mode === 'text') {
      out[q.id] = a?.text?.trim() || null;
    } else if (q.mode === 'single') {
      out[q.id] = a?.selected?.[0] ?? (q.allowsOther ? other : null);
    } else {
      const picked = [...(a?.selected ?? []), ...(q.allowsOther && other ? [other] : [])];
      out[q.id] = picked.length ? picked : null;
    }
  }
  return out;
}

// ── The open remote chat ─────────────────────────────────────────────────

const [chat, setChat] = createSignal<RemoteChat | null>(null);
let stopEvents: (() => void) | null = null;

/** The open remote chat, or null. */
export const remoteChat = chat;

const messageId = () => crypto.randomUUID();

/** PL1: the id a send got, kept while its link dropped (`offline`), so the
 *  person's retry of the same text in the same chat sends the same command.
 *  The other Mac saves a send under its id before it answers and gives a
 *  repeat that answer (for 10 minutes), so a send that landed before the link
 *  died is not queued twice. Any answer, or a refusal, forgets it. */
const unsent = new Map<string, string>();
const UNSENT_KEPT = 50;

function sendIdFor(slot: string): string {
  const kept = unsent.get(slot);
  if (kept) return kept;
  const id = messageId();
  unsent.set(slot, id);
  if (unsent.size > UNSENT_KEPT) unsent.delete(unsent.keys().next().value as string);
  return id;
}

/** Each open (and each close) gets a new token: a late answer for an
 *  earlier open of the same chat is told apart by it, not by Mac and key. */
let token = 0;
const current = (t: number) => t === token && chat() !== null;

/** A command's failure: its PeerFailure, and an offline one marks the chat
 *  it was for, only while that open of it is still the one shown. */
function failed(e: unknown, t: number): PeerFailure | null {
  if (!isPeerFailure(e)) return null;
  if (e.state === 'offline' && current(t)) setChat(c => (c ? { ...c, offline: e.message } : c));
  return e;
}

/** How long a watch or unwatch may take before the next one goes anyway. */
export const STREAM_CALL_TIMEOUT_MS = 15_000;

/** The chat stream's watch and unwatch calls, one after another, so an
 *  unwatch never overtakes the watch it ends (the backend would keep that
 *  stream open: nothing about a closed chat may keep coming). The chain
 *  swallows failures so the next call always goes; the caller gets the call's
 *  own result, and a call that hangs fails after STREAM_CALL_TIMEOUT_MS. */
let streamCalls: Promise<unknown> = Promise.resolve();
function inOrder(command: 'peer_watch' | 'peer_unwatch', machineId: string, key: string): Promise<unknown> {
  const call = streamCalls.then(() => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject({
      code: 'offline', message: 'The other Mac didn’t answer within 15 seconds.', state: 'offline', attemptsLeft: null,
    } satisfies PeerFailure), STREAM_CALL_TIMEOUT_MS);
    invoke(command, { machineId, key }).then(resolve, reject).finally(() => clearTimeout(timer));
  }));
  streamCalls = call.catch(() => {});
  return call;
}

function detailInto(t: number, d: PeerChatDetail): void {
  if (current(t)) setChat(c => (c ? withDetail(c, d) : c));
}

/** Open `key` on `machineId`: its turns, then its stream. A chat closed (or
 *  opened again) while its turns load is never watched by this open; a
 *  watch that fails shows why (offline). */
export async function openRemoteChat(machineId: string, key: string): Promise<void> {
  closeRemoteChat();
  const t = ++token;
  setChat(emptyRemoteChat(machineId, key));
  stopEvents = onNative<PeerChatEvent>('peer-chat-event', e => setChat(c => (c ? foldPeerChat(c, e) : c)), () => {});
  try {
    const d = await invoke<PeerChatDetail>('peer_detail', { machineId, key, before: null, limit: 20 });
    if (!current(t)) return;
    detailInto(t, d);
    await inOrder('peer_watch', machineId, key);
  } catch (e) {
    failed(e, t);
  }
}

/** Fetch the open chat's newest turns again (after an answer, or by hand). */
export async function refreshRemoteChat(): Promise<void> {
  const c = chat();
  if (!c) return;
  const t = token;
  try {
    detailInto(t, await invoke<PeerChatDetail>('peer_detail', { machineId: c.machineId, key: c.key, before: null, limit: 20 }));
  } catch (e) {
    failed(e, t);
  }
}

/** Load the page before the oldest turn held. */
export async function loadEarlierRemoteTurns(): Promise<void> {
  const c = chat();
  if (!c || !c.hasMore || !c.turns.length) return;
  const t = token;
  try {
    const d = await invoke<PeerChatDetail>('peer_detail', { machineId: c.machineId, key: c.key, before: c.turns[0].ord, limit: 20 });
    if (current(t)) setChat(now => (now ? withEarlier(now, d) : now));
  } catch (e) {
    failed(e, t);
  }
}

/** Close the view: its stream stops and its turns are dropped. */
export function closeRemoteChat(): void {
  const c = chat();
  token++;
  stopEvents?.();
  stopEvents = null;
  setChat(null);
  if (c) void inOrder('peer_unwatch', c.machineId, c.key).catch(() => {});
}

/** Send the open chat a message at its home. Rejects with the PeerFailure
 *  (busy keeps the draft; offline says why). */
export async function sendRemote(text: string): Promise<Accepted> {
  const c = chat();
  if (!c) throw new Error('No remote chat is open.');
  const t = token;
  const slot = `${c.machineId}\n${c.key}\n${text}`;
  const id = sendIdFor(slot);
  try {
    const accepted = await invoke<Accepted>('peer_send', { machineId: c.machineId, key: c.key, text, clientMessageId: id });
    unsent.delete(slot);
    return accepted;
  } catch (e) {
    if (!isPeerFailure(e) || e.code !== 'offline') unsent.delete(slot);
    throw failed(e, t) ?? e;
  }
}

/** Stop the open chat's turn at its home. */
export async function stopRemote(): Promise<boolean> {
  const c = chat();
  if (!c) return false;
  const t = token;
  try {
    return await invoke<boolean>('peer_stop', { machineId: c.machineId, key: c.key });
  } catch (e) {
    throw failed(e, t) ?? e;
  }
}

/** Answer the open chat's card at its home. A card closed there meanwhile
 *  (`question-closed`) reloads the chat. */
export async function answerRemote(questions: WireQuestions, answer: InteractionAnswer): Promise<Accepted> {
  const c = chat();
  if (!c) throw new Error('No remote chat is open.');
  const t = token;
  try {
    const accepted = await invoke<Accepted>('peer_answer', {
      machineId: c.machineId, key: c.key, questionId: questions.id, clientMessageId: messageId(), answers: cardAnswerToWire(questions, answer),
    });
    // The card closed there: fetch the chat now rather than wait for its
    // event (a dropped one would leave the card open here).
    if (current(t)) void refreshRemoteChat();
    return accepted;
  } catch (e) {
    const f = failed(e, t);
    if (f?.code === 'question-closed' && current(t)) void refreshRemoteChat();
    throw f ?? e;
  }
}
