// Mid-turn steering in the composer (2026-10-21; backend `steering.rs`,
// docs/reference/agents.md "Mid-turn steering"). While a turn runs, a send
// is one of three choices:
// - Steer now: the running agent takes it at its next step, in this turn
//   (agents whose transport has a true steer: `chat_steer_support`).
// - Queue for after: it goes as the next turn when this one ends.
// - Stop and send: the turn stops, then it goes, resuming the same session.
// The composer says which a plain Enter does before anything is pressed.
import type { ChatTurn } from './chatTurnTypes';

export type SteerState = 'sent' | 'delivered' | 'notDelivered';

/** One steer as the backend sends and stores it (`steering::SteerRecord`). */
export interface SteerWire {
  id: string;
  text: string;
  /** RFC 3339, when the person sent it. */
  at: string;
  /** Tool calls the turn had made when it was sent. */
  afterTools: number;
  state: SteerState;
  /** The agent's own words when it refused the steer (Codex, P1-30). */
  reason?: string | null;
}

/** `chat_steer_support`: what Steer now does for one agent. */
export interface SteerSupportRow {
  agent: string;
  support: 'native' | 'none';
  note: string;
}

/** A send held for after the running turn (`chat_send` with `after`). */
export interface HeldChip {
  id: string;
  text: string;
  after: 'queue' | 'stop';
  /** Images sent with it (they come back on Cancel or a failure). */
  attachments?: import('./store/chatRuntime').ChatAttachment[];
}

export type BusyChoice = 'steer' | 'queue' | 'stop';

/** The backend's limits (`steering::MAX_STEER_CHARS`, `MAX_STEER_JSON`,
 *  `MAX_STEERS_PER_TURN`). */
export const MAX_STEER_CHARS = 3_000;
export const MAX_STEER_JSON = 7_000;
export const MAX_STEERS_PER_TURN = 20;

/** Whether the backend would refuse `text` as too long: over 3,000
 *  characters, or over 7,000 bytes as JSON (escapes and non-Latin text). */
export function steerTooLong(text: string): boolean {
  const t = text.trim();
  // Each UTF-16 unit is at most 6 bytes as JSON: short drafts skip the count.
  if (t.length * 6 + 2 <= MAX_STEER_JSON) return false;
  return [...t].length > MAX_STEER_CHARS || new TextEncoder().encode(JSON.stringify(t)).length > MAX_STEER_JSON;
}

/** What rules Steer now out besides the agent: an image, an attached file
 *  (C5), a shared page's text (repo ideas 2026-10-27 #3), a draft too long
 *  (`chars` past MAX_STEER_CHARS, or `tooLong`), a turn still queued,
 *  support not read yet. */
export interface SteerBlock { images?: boolean; files?: boolean; pages?: boolean; chars?: number; tooLong?: boolean; queued?: boolean; unknown?: boolean }

/** Why Enter can't steer right now, in words for the hint, or '' (P1-30). */
export function steerBlockedWhy(agent: string, native: boolean, b: SteerBlock = {}): string {
  if (b.queued) return `${agent}'s turn hasn't started yet`;
  if (b.unknown) return `checking whether ${agent} can take a message mid-turn`;
  if (!native) return `${agent} can't take a message mid-turn`;
  if (b.images) return "an image can't go into a running turn";
  if (b.files) return "an attached file can't go into a running turn";
  if (b.pages) return "shared page text can't go into a running turn";
  if (b.tooLong || (b.chars ?? 0) > MAX_STEER_CHARS) return 'too long to steer: the limit is 3,000 characters, fewer for non-Latin text';
  return '';
}

/** What a plain Enter does while a turn runs: Steer now where the agent
 *  takes messages mid-turn and nothing rules it out, else Queue for after. */
export function defaultBusyChoice(native: boolean, hasImages: boolean, more: Omit<SteerBlock, 'images'> = {}): BusyChoice {
  return steerBlockedWhy('', native, { ...more, images: hasImages }) ? 'queue' : 'steer';
}

/** A `chat_steer` refusal in words (P1-30): what happened, and whether the
 *  message goes as the next one (`fallsBack`) or back to the box. Never the
 *  code itself. */
export function steerRefusalNotice(code: string, agent: string): { text: string; fallsBack: boolean } {
  const next = 'so your message goes as the next one instead.';
  if (code.includes('STEER_NOT_RUNNING')) return { fallsBack: true, text: `The turn hasn't started yet (or just ended), ${next}` };
  if (code.includes('STEER_ENDING')) return { fallsBack: true, text: `The turn was finishing, ${next}` };
  if (code.includes('STEER_UNSUPPORTED')) return { fallsBack: true, text: `${agent} can't take a message mid-turn, ${next}` };
  if (code.includes('STEER_INVALID')) return { fallsBack: false, text: `Your steer wasn't sent: it was too long (the limit is ${MAX_STEER_CHARS.toLocaleString('en-US')} characters, fewer for non-Latin text), or this turn already took ${MAX_STEERS_PER_TURN} messages mid-turn.` };
  if (code.includes('STEER_NOT_STORED')) return { fallsBack: false, text: "Your steer wasn't sent: Hollow couldn't save it to this conversation." };
  if (code.includes('STEER_')) return { fallsBack: false, text: "Your steer wasn't sent." };
  return { fallsBack: false, text: `Your steer wasn't sent: ${code.replace(/^Error:\s*/, '').trim().replace(/[.\s]+$/, '')}.` };
}

/** The send button's label and the hint above the box, per choice. */
export function busyChoiceLabel(choice: BusyChoice): string {
  return choice === 'steer' ? 'Steer now' : choice === 'queue' ? 'Queue for after' : 'Stop and send';
}

/** One line for the menu and the hint: what happens, for this agent. */
export function busyChoiceDetail(choice: BusyChoice, agent: string): string {
  switch (choice) {
    case 'steer': return `${agent} takes your message at its next step, in this turn.`;
    case 'queue': return 'Sends it as the next message when this turn ends.';
    case 'stop': return `Stops ${agent}, then sends it, picking up the same session.`;
  }
}

/** The hint above the box while a turn runs (announced politely). `key`
 *  is the send key the person set (Enter, or ⌘Enter when Enter is a new line);
 *  `stopKey` the stop-run chord as shown, '' for none. The chat passes a ⌘
 *  chord only: this line shows while the box holds text, when Esc doesn't
 *  stop (FIX-ESC).
 *  `why` is `steerBlockedWhy`'s reason, said when Enter queues (P1-30). */
export function busyHint(choice: BusyChoice, agent: string, why: string, key = 'Enter', stopKey = ''): string {
  const enter = choice === 'steer' ? `${key} steers ${agent} at its next step` : `${key} queues it for after this turn`;
  return `${enter}${why && choice === 'queue' ? ` (${why})` : ''}${stopKey ? ` · ${stopKey} stops` : ''} · Send options has Queue for after and Stop and send`;
}

/** The composer's placeholder while a turn runs and the box is empty
 *  (P1-10): what Enter will do and how to stop, in one line. `stopKey` is the
 *  stop-run chord as shown, '' when it has none. */
export function busyPlaceholder(choice: BusyChoice, agent: string, _native: boolean, key: string, stopKey: string): string {
  // The same words as busyHint's first half; why a queue can't steer is
  // the typed hint's to say.
  const enter = choice === 'steer' ? `${key} steers ${agent} at its next step` : `${key} queues it for after this turn`;
  return `${enter} · ${stopKey ? `${stopKey} stops this turn` : 'Stop ends this turn'}`;
}

/** Stop's tooltip: its chord when it has one (P1-10), never a key that is off. */
export function stopTitle(stopKey: string): string {
  return stopKey ? `Stop (${stopKey})` : 'Stop';
}

/** A steer's state in words, beside it in the chat; a refusal in the
 *  agent's own words when it gave them (P1-30). */
export function steerStateText(state: SteerState, reason?: string | null): string {
  switch (state) {
    case 'sent': return 'Waiting for the agent’s next step';
    case 'delivered': return 'Taken into this turn';
    case 'notDelivered': return reason ? `Not taken: the agent said “${reason}”` : 'Not taken: the turn ended first';
  }
}

/** A steer as a transcript row: the person's words, inside the running
 *  reply (a quiet system row, so a finished reply's fold keeps it visible). */
export function steerTurn(steer: SteerWire): ChatTurn {
  return { role: 'system', text: steer.text, tools: [], notice: 'steer', steer: { id: steer.id, at: steer.at, state: steer.state, afterTools: steer.afterTools, ...(steer.reason ? { reason: steer.reason } : {}) } };
}

/** "2:42 PM": when a steer was sent, in the person's clock. */
export function steerTime(at: string): string {
  const when = new Date(at);
  return Number.isNaN(when.getTime()) ? '' : when.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

/** A restored assistant row's steers, as rows to show before its answer. */
export function steerRowsOf(turn: { steers?: SteerWire[] | null }): ChatTurn[] {
  return (turn.steers ?? []).map(steerTurn);
}

/** A composer-minted steer id (`steering::is_safe_id`). */
export function newSteerId(): string {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  return `steer-${Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('')}`;
}

/** The refusal code in a rejected `chat_steer` (Tauri rejects with the string). */
export function steerRefusal(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** The Send options menu's rows (`ActionMenu` items): Steer now (or, for an
 *  agent without a true steer, a plain line saying so), Queue for after and
 *  Stop and send, each with what it does. A Steer now that something rules
 *  out says why (`blocked`, else an image); an agent without one reads the
 *  backend's own note when it has one (P1-30). */
export function busyMenuItems(agent: string, native: boolean, opts: { empty: boolean; images: boolean; note?: string; blocked?: string }, pick: (choice: BusyChoice) => void) {
  const blocked = opts.blocked || (opts.images ? steerBlockedWhy(agent, native, { images: true }) : '');
  const item = (choice: BusyChoice) => ({
    label: `${busyChoiceLabel(choice)}: ${choice === 'steer' && blocked ? blocked : busyChoiceDetail(choice, agent)}`,
    action: () => pick(choice),
    disabled: opts.empty || (choice === 'steer' && !!blocked),
  });
  const steer = native
    ? [item('steer')]
    : [{ label: opts.note || `${agent} can't take a message mid-turn: queue it, or Stop and send`, action: () => {}, informational: true }];
  return [...steer, item('queue'), { ...item('stop'), danger: true }];
}

