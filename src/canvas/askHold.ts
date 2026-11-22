/** Ask-canvas answers Hollow is holding (P1-33), by conversation and canvas
 *  revision, so every copy of that revision (the transcript's and the side
 *  panel's) shows the same thing and sends once:
 *  - `sending`: a Send from one copy is on its way; every copy's Send is off.
 *  - `queued`: Send was pressed while the agent was answering, and the
 *    answers wait for after the turn (Queue for after, never Steer) as held
 *    send `held`. Send stays off, so a second press sends nothing.
 *  - `not-sent`: a Send that didn't go. The answers stay out of the message
 *    box; the canvas offers Send now and Take them back.
 *  Nothing is kept across a reload: a held send that goes or fails reports
 *  through its chip, and the answers themselves stay in the canvas's state. */
import { createRoot, createSignal } from 'solid-js';

export type AskHold = { kind: 'sending' } | { kind: 'queued'; held: string } | { kind: 'not-sent'; text: string };

const [holds, setHolds] = createRoot(() => createSignal<Record<string, AskHold>>({}));

/** The hold's key: a conversation's canvas revision (`answerKey`). */
export const holdKey = (sessionKey: string | null, answer: string) => `${sessionKey ?? ''}|${answer}`;

export function askHold(key: string): AskHold | null {
  return holds()[key] ?? null;
}

export function setAskHold(key: string, hold: AskHold | null) {
  setHolds((all) => {
    const next = { ...all };
    if (hold) next[key] = hold;
    else delete next[key];
    return next;
  });
}

/** A held send that went or failed (`heldSent`/`heldFailed`, or its chip's
 *  Cancel): the hold that waits on it ends, a failure as `not-sent` with its
 *  text. Returns whether one did. */
export function settleHeldAnswers(held: string, failedText: string | null): boolean {
  const key = Object.keys(holds()).find(k => { const h = holds()[k]; return h.kind === 'queued' && h.held === held; });
  if (!key) return false;
  setAskHold(key, failedText === null ? null : { kind: 'not-sent', text: failedText });
  return true;
}

const ANSWER = /```hollow-canvas-answer\n(\{[^\n]*\})\n```/;

/** The canvas revision (`answerKey`) an ask-canvas answers message answers,
 *  else null (the hidden block `canvasAnswerText` writes). */
export function answeredRevision(text: string): string | null {
  const m = ANSWER.exec(text);
  if (!m) return null;
  try {
    const a = JSON.parse(m[1]) as { canvasId?: unknown; revision?: unknown };
    return typeof a.canvasId === 'string' && Number.isInteger(a.revision) ? `${a.canvasId}@${a.revision}` : null;
  } catch { return null; }
}

/** An answers message that came back unsent (a queued turn stopped or ended
 *  before it started): its canvas shows Send now and Take them back instead
 *  of the text going to the message box. Returns whether `text` was one. */
export function answersBack(sessionKey: string | null, text: string): boolean {
  const answer = answeredRevision(text);
  if (!answer) return false;
  setAskHold(holdKey(sessionKey, answer), { kind: 'not-sent', text });
  return true;
}
