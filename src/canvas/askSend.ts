/** Ask-canvas Send (question cards, 2026-10-14): what Hollow says under a
 *  canvas when its Send can't go, and the Send button's words. Pure, so the
 *  node suite reads it directly. */
import type { DraftRefusal } from '../canvasBridge';

export interface AskSendStatus {
  text: string;
  tone: 'info' | 'success' | 'warning' | 'danger';
  /** Offer Try again. */
  retry?: boolean;
}

/** One plain line per refusal. */
export function askRefusalStatus(r: DraftRefusal): AskSendStatus {
  switch (r) {
    case 'locked': return { text: 'Already answered.', tone: 'success' };
    case 'not-current': return { text: 'Only the newest saved version of this canvas can send its answers. Reply in the message box instead.', tone: 'warning' };
    case 'unavailable': return { text: 'Answers can’t be sent from here. Open the conversation to answer.', tone: 'warning' };
    // While the agent answers, Send queues (P1-33): busy is a send of these
    // answers already going (from the side panel's copy, say) or a
    // conversation still opening.
    case 'busy': return { text: 'These answers are already on their way, or the conversation is still opening. Try again in a moment.', tone: 'warning', retry: true };
    // Hollow took the answers but couldn't send them: the canvas keeps them
    // (P1-33; they went to the message box before).
    case 'not-sent': return NOT_SENT_STATUS;
    case 'send-failed': return { text: 'Your answers weren’t sent. Try again.', tone: 'warning', retry: true };
    default: return { text: 'Hollow couldn’t read these answers.', tone: 'danger', retry: true };
  }
}

/** P1-33: the Send button's words while the agent is answering, and what
 *  the canvas says once the answers wait for after the turn, or didn't go. */
export const QUEUE_LABEL = 'Queue answers';
export const SENDING_STATUS: AskSendStatus = { text: 'Sending your answers…', tone: 'info' };
export const QUEUED_STATUS: AskSendStatus = { text: 'Queued. Your answers go as soon as this turn ends.', tone: 'info' };
export const NOT_SENT_STATUS: AskSendStatus = { text: 'Your answers weren’t sent. Send them now, or take them back to change them.', tone: 'warning' };

/** A Send click in the half second after the frame changed height, which
 *  moves the button (2026-10-15): it may have slid under a pointer aimed at
 *  something else, so nothing is collected. */
export const MOVED_STATUS: AskSendStatus = { text: 'The canvas changed size just as you clicked, so nothing was sent. Click again to send.', tone: 'info' };

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: '\'', '#39': '\'' };
export const DEFAULT_SEND_LABEL = 'Send answers';

/** The first `<hc-ask send="…">` in a canvas's inner HTML, as plain text
 *  (at most 40 characters), else "Send answers". */
export function askSendLabel(inner: string): string {
  const m = /<hc-ask\b[^>]*?\ssend\s*=\s*("([^"]*)"|'([^']*)')/i.exec(inner);
  const raw = (m?.[2] ?? m?.[3] ?? '').replace(/&(amp|lt|gt|quot|apos|#39);/g, (_, e: string) => ENTITIES[e]).replace(/\s+/g, ' ').trim();
  return raw ? [...raw].slice(0, 40).join('') : DEFAULT_SEND_LABEL;
}
