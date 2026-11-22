/**
 * An ask-canvas's answer in the conversation (Canvas C4 A2, plan §5). Pure,
 * so the node Vitest suite covers it.
 *
 * **What a draft puts in the composer** (`canvasAnswerText`): the header
 * `Answered canvas "<id>":`, one `- label: value` line per line of the
 * draft's summary (the line shape `format_answers` gives a question card's
 * answers, where a card's own prompt may run to PROMPT_MAX rather than this
 * 200; `answerLine` and Rust's `push_line` are both tested against
 * `answerLines.golden.json`), then the exact answers as one line of
 * JSON in a fenced `hollow-canvas-answer` block:
 * `{"canvasId", "revision", "answers"}`. The person sees all of it in the
 * composer and sends it with Enter; nothing sends it for them.
 *
 * **Hidden in the transcript, read by the agent.** The block follows the
 * `hollow-next` convention (`splitHiddenFence` in `chatSegments.ts`): the
 * user bubble shows the text without it (`visibleUserText`), Rust strips it
 * from vault session records (`next_step::strip`), and the agent's prompt
 * keeps it, so the agent reads the JSON.
 *
 * **The lock lives in the transcript** (`answeredCanvases`): a user message
 * holding a valid block answers that canvas revision. Nothing is written
 * anywhere when the person sends: the conversation's own record keeps the
 * message, so the lock survives a restart and reopening the conversation,
 * an unsent or withdrawn message never locks, and a later revision of the
 * same canvas id is a new question.
 */

import { splitHiddenFence } from '../chatSegments';
import { CANVAS_ID, LIMITS, draftAnswers, type DraftAnswers } from '../canvasBridge';

/** The fence language of an ask-canvas's answers in a user message. Rust:
 *  `next_step::CANVAS_ANSWER_LANG`. */
export const ANSWER_FENCE = 'hollow-canvas-answer';

/** One answered ask-canvas, as the hidden block holds it. */
export interface CanvasAnswer { canvasId: string; revision: number; answers: DraftAnswers }

const WS = /\p{White_Space}+/u;
const WS_ENDS = /^\p{White_Space}+|\p{White_Space}+$/gu;
const CONTROL = /\p{Cc}/gu;
/** `interaction_schema::LABEL_MAX`, in characters (code points). */
const LABEL_MAX = 200;

/** Rust `one_line_capped` at `LABEL_MAX`: whitespace collapsed, control
 *  characters dropped, at most 200 characters. */
function oneLine(label: string): string {
  const collapsed = label.split(WS).filter(Boolean).join(' ').replace(CONTROL, '');
  return Array.from(collapsed).slice(0, LABEL_MAX).join('');
}

/** Rust `clean_value`: per line, tabs to spaces, control characters dropped,
 *  the end trimmed. */
function cleanValue(value: string): string {
  const lines = value.split('\n');
  // Rust's `lines()` yields no empty last line for a trailing newline.
  if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
  return lines.map(l => l.replace(/\t/g, ' ').replace(CONTROL, '').replace(/\p{White_Space}+$/u, '')).join('\n');
}

/** One answer line, `- label: value`, exactly as `push_line` writes it in
 *  `format_answers` (without the newline before it). */
export function answerLine(label: string, value: string): string {
  const [first = '', ...rest] = cleanValue(value).replace(WS_ENDS, '').split('\n');
  return `- ${oneLine(label)}: ${first}${rest.map(l => `\n  ${l}`).join('')}`;
}

/** The composer text for a draft: header, readable lines, hidden JSON. */
export function canvasAnswerText(answer: CanvasAnswer, summary: string): string {
  const lines = summary.split('\n').filter(l => l.trim()).map((l) => {
    const at = l.indexOf(': ');
    return at > 0 ? answerLine(l.slice(0, at), l.slice(at + 2)) : `- ${oneLine(l)}`;
  });
  const json = JSON.stringify({ canvasId: answer.canvasId, revision: answer.revision, answers: answer.answers });
  return `Answered canvas "${answer.canvasId}":\n${lines.join('\n')}\n\n\`\`\`${ANSWER_FENCE}\n${json}\n\`\`\``;
}

const isPlain = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v) && Object.getPrototypeOf(v) === Object.prototype;

/** A block's JSON as an answer, or null when it is not exactly one. */
function parseAnswer(json: string): CanvasAnswer | null {
  let v: unknown;
  try { v = JSON.parse(json); } catch { return null; }
  if (!isPlain(v)) return null;
  const keys = Object.keys(v);
  if (keys.length !== 3 || !['canvasId', 'revision', 'answers'].every(k => keys.includes(k))) return null;
  const { canvasId, revision } = v;
  if (typeof canvasId !== 'string' || !CANVAS_ID.test(canvasId)) return null;
  if (typeof revision !== 'number' || !Number.isInteger(revision) || revision < 1) return null;
  const answers = draftAnswers(v.answers);
  return answers ? { canvasId, revision, answers } : null;
}

/** Every valid answer block in a message, in order. */
export function readCanvasAnswers(text: string): CanvasAnswer[] {
  return splitHiddenFence(text ?? '', ANSWER_FENCE).blocks
    .map(lines => parseAnswer(lines.join('\n').trim()))
    .filter((a): a is CanvasAnswer => a !== null);
}

/** A user message as the transcript shows it: without its answer blocks. */
export function visibleUserText(text: string): string {
  return splitHiddenFence(text ?? '', ANSWER_FENCE).text;
}

export const answerKey = (canvasId: string, revision: number) => `${canvasId}@${revision}`;

/** A locked ask-canvas revision: the answer, the visible lines the person
 *  sent (without the header, at most `LIMITS.answeredChars`), and the turn. */
export interface AnsweredCanvas { answer: CanvasAnswer; summary: string; turn: number }

const HEADER = /^Answered canvas "[a-z0-9-]{1,64}":[ \t]*\n?/;

/** Which canvas revisions this transcript has answered, by `answerKey`. The
 *  first answer to a revision is the one that locked it. */
export function answeredCanvases(turns: readonly { role: string; text: string; clientTurnId?: string; queueId?: number; runId?: string; status?: string | null }[]): Map<string, AnsweredCanvas> {
  const out = new Map<string, AnsweredCanvas>();
  turns.forEach((t, turn) => {
    if (t.role !== 'user' || !t.text?.includes(ANSWER_FENCE)) return;
    // The local bubble appears before chat_send accepts it. A failed send
    // removes that bubble, but a frame's answered lock cannot be undone.
    // Wait for queue/run evidence; legacy restored messages have no nonce.
    if (t.clientTurnId && t.queueId == null && !t.runId && !t.status) return;
    const answers = readCanvasAnswers(t.text);
    if (!answers.length) return;
    const summary = visibleUserText(t.text).replace(HEADER, '').trim().slice(0, LIMITS.answeredChars);
    for (const answer of answers) {
      const key = answerKey(answer.canvasId, answer.revision);
      if (!out.has(key)) out.set(key, { answer, summary, turn });
    }
  });
  return out;
}
