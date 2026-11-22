/**
 * Structured question cards (Paperclip PR 13): the pure half of the composer's
 * card handling. The backend (`interactions.rs`) is the authority on validity
 * and staleness; these helpers keep the form honest before Submit. What a
 * failed answer turn does with the answers, and the shown list, live in
 * questionCardList.ts (re-exported here).
 *
 * Type-only imports besides that pure module: the node vitest suite loads
 * this module directly.
 */
import type {
  ChatInteraction,
  InteractionAnswer,
  InteractionQuestion,
  InteractionRequest,
  InteractionVerdict,
} from './types';

export { ANSWERS_IN_COMPOSER_NOTICE } from './questionCardList';

export interface QuestionFormAnswer {
  selected: string[];
  /** The custom "Other" answer is switched on. */
  otherOn: boolean;
  other: string;
  text: string;
}

export interface QuestionFormState {
  answers: Record<string, QuestionFormAnswer>;
  accepted: boolean | null;
  note: string;
  selected: string[];
  verdicts: Record<string, { verdict: InteractionVerdict | null; reason: string }>;
}

function questionsOf(request: InteractionRequest): InteractionQuestion[] {
  return request.mode === 'questions' ? (request.questions ?? []) : [];
}

/** A fresh form for a card: nothing chosen, checkbox defaults preselected. */
export function initialFormState(request: InteractionRequest): QuestionFormState {
  const answers: Record<string, QuestionFormAnswer> = {};
  for (const q of questionsOf(request)) {
    answers[q.id] = { selected: [], otherOn: false, other: '', text: '' };
  }
  const verdicts: QuestionFormState['verdicts'] = {};
  for (const item of request.verdicts?.items ?? []) {
    verdicts[item.id] = { verdict: null, reason: '' };
  }
  return {
    answers,
    accepted: null,
    note: '',
    selected: request.mode === 'checkbox' ? [...(request.checkbox?.defaultSelected ?? [])] : [],
    verdicts,
  };
}

/** The question's short label on the card (its trail and its fields): the
 *  header, else the prompt. The answers name the whole prompt instead
 *  (`format_answers`, 2026-11-04). */
export function questionLabel(q: InteractionQuestion): string {
  return q.header?.trim() ? q.header : q.prompt;
}

/** The one line under a question: `reason` when the card has one, else the
 *  help text collapsed to a single line. Empty when neither is set. */
export function questionReason(q: { reason?: string | null; helpText?: string | null }): string {
  const raw = q.reason?.trim() ? q.reason : (q.helpText ?? '');
  return raw.replace(/\s+/g, ' ').trim();
}

/** A decision question's scenario as one plain line of text (the card draws
 *  it as text, never markdown). */
export function questionScenario(q: { scenario?: string | null }): string {
  return (q.scenario ?? '').replace(/\s+/g, ' ').trim();
}

/** What an option does in the scenario, as one plain line, or ''. */
export function optionOutcome(o: { outcome?: string | null }): string {
  return (o.outcome ?? '').replace(/\s+/g, ' ').trim();
}

function textError(q: InteractionQuestion, raw: string): string | null {
  const text = raw.trim();
  if (!text) return q.required !== false ? 'Required' : null;
  const tv = q.textValidation;
  if (!tv) return null;
  const n = [...text].length;
  if (tv.minLength != null && n < tv.minLength) return `At least ${tv.minLength} characters`;
  if (tv.maxLength != null && n > tv.maxLength) return `At most ${tv.maxLength} characters`;
  if (tv.pattern) {
    try {
      if (!new RegExp(`^(?:${tv.pattern})$`).test(text)) return 'Does not match the expected format';
    } catch {
      // The backend's regex dialect differs slightly; it decides on Submit.
    }
  }
  if (tv.inputType === 'number' || tv.inputType === 'integer') {
    const value = Number(text);
    if (!Number.isFinite(value) || (tv.inputType === 'integer' && !Number.isInteger(value))) {
      return tv.inputType === 'integer' ? 'Enter a whole number' : 'Enter a number';
    }
    if ((tv.minimum != null && value < tv.minimum) || (tv.maximum != null && value > tv.maximum)) {
      return 'Out of range';
    }
  }
  return null;
}

/** Per-field problems keyed by question id, `confirmation`, `checkbox` or
 *  verdict item id. Empty means the form may be submitted. */
export function formErrors(request: InteractionRequest, state: QuestionFormState): Record<string, string> {
  const errors: Record<string, string> = {};
  switch (request.mode) {
    case 'questions':
      for (const q of questionsOf(request)) {
        const a = state.answers[q.id] ?? { selected: [], otherOn: false, other: '', text: '' };
        if (q.answerMode === 'text') {
          const error = textError(q, a.text);
          if (error) errors[q.id] = error;
          continue;
        }
        const other = a.otherOn && q.allowCustom ? a.other.trim() : '';
        if (a.otherOn && q.allowCustom && !other) errors[q.id] = 'Type your answer';
        else if (q.required !== false && a.selected.length === 0 && !other) errors[q.id] = 'Required';
      }
      break;
    case 'confirmation':
      if (state.accepted == null) errors.confirmation = 'Choose one';
      break;
    case 'checkbox': {
      const c = request.checkbox;
      const n = state.selected.length;
      if (c?.minSelected != null && n < c.minSelected) errors.checkbox = `Select at least ${c.minSelected}`;
      else if (c?.maxSelected != null && n > c.maxSelected) errors.checkbox = `Select at most ${c.maxSelected}`;
      break;
    }
    case 'verdicts': {
      const v = request.verdicts;
      for (const item of v?.items ?? []) {
        const given = state.verdicts[item.id];
        if (!given?.verdict) errors[item.id] = 'Choose a verdict';
        else if ((v?.requireReasonOn ?? []).includes(given.verdict) && !given.reason.trim()) errors[item.id] = 'Give a reason';
      }
      break;
    }
  }
  return errors;
}

/** The answer `resolve_interaction` receives. Only the mode's own fields. */
export function buildAnswer(request: InteractionRequest, state: QuestionFormState): InteractionAnswer {
  switch (request.mode) {
    case 'questions': {
      const answers: NonNullable<InteractionAnswer['answers']> = {};
      for (const q of questionsOf(request)) {
        const a = state.answers[q.id];
        if (!a) continue;
        if (q.answerMode === 'text') {
          if (a.text.trim()) answers[q.id] = { text: a.text.trim() };
          continue;
        }
        const entry: { selected?: string[]; other?: string } = {};
        if (a.selected.length) entry.selected = q.answerMode === 'single' ? a.selected.slice(0, 1) : [...a.selected];
        if (a.otherOn && q.allowCustom && a.other.trim()) entry.other = a.other.trim();
        if (q.answerMode === 'single' && entry.other) delete entry.selected;
        if (entry.selected || entry.other) answers[q.id] = entry;
      }
      return { answers };
    }
    case 'confirmation': {
      const answer: InteractionAnswer = { accepted: state.accepted === true };
      if (state.note.trim()) answer.note = state.note.trim();
      return answer;
    }
    case 'checkbox':
      return { selected: [...state.selected] };
    case 'verdicts': {
      const verdicts: NonNullable<InteractionAnswer['verdicts']> = {};
      for (const [id, given] of Object.entries(state.verdicts)) {
        if (!given.verdict) continue;
        verdicts[id] = given.reason.trim() ? { verdict: given.verdict, reason: given.reason.trim() } : { verdict: given.verdict };
      }
      const answer: InteractionAnswer = { verdicts };
      if (state.note.trim()) answer.note = state.note.trim();
      return answer;
    }
  }
}

/** Approve every item that has no verdict yet ("Approve all"). */
export function approveAll(request: InteractionRequest, state: QuestionFormState): QuestionFormState {
  if (request.mode !== 'verdicts' || !(request.verdicts?.verdicts ?? []).includes('approve')) return state;
  const verdicts = { ...state.verdicts };
  for (const item of request.verdicts?.items ?? []) {
    const given = verdicts[item.id];
    if (!given?.verdict) verdicts[item.id] = { verdict: 'approve', reason: given?.reason ?? '' };
  }
  return { ...state, verdicts };
}

/** The card's heading. */
export function cardTitle(card: Pick<ChatInteraction, 'request'>): string {
  const title = card.request.title?.trim();
  if (title) return title;
  switch (card.request.mode) {
    case 'confirmation': return 'Confirm';
    case 'checkbox': return 'Choose';
    case 'verdicts': return 'Review';
    default: {
      const n = card.request.questions?.length ?? 0;
      return n === 1 ? 'A question from the agent' : 'Questions from the agent';
    }
  }
}

/** Why Send is disabled right now, or null when it may be pressed. The card
 *  shows it from its first question, before any answer is picked. The
 *  session that asked never blocks it (2026-11-04): when the conversation
 *  has moved on to another native session, the answers go as an ordinary
 *  message instead ([`sendsAsMessage`]). */
export function submitBlockReason(input: {
  card: Pick<ChatInteraction, 'status'>;
  busy: boolean;
  loading: boolean;
}): string | null {
  const { card } = input;
  if (card.status === 'stale') return 'The file this question is about changed, so the card can no longer be answered.';
  if (card.status !== 'pending') return 'This card is closed.';
  if (input.loading) return 'The conversation is still loading.';
  if (input.busy) return 'Answers can be sent once the current turn finishes.';
  return null;
}

/** Whether the answers go as an ordinary message: the conversation no
 *  longer resumes the native session that asked (a Main check-in's card
 *  after the next turn started a fresh one). The backend words each
 *  question in full either way (`format_answers`). */
export function sendsAsMessage(card: Pick<ChatInteraction, 'nativeSessionId'>, currentNative: string | null): boolean {
  return !card.nativeSessionId || card.nativeSessionId !== currentNative;
}

/** The card's note when its answers go as a message: the session that
 *  asked has gone on, or (an Antigravity card) never could be resumed. */
export const AS_MESSAGE_NOTE = 'This chat can’t resume the session that asked, so your answers go as a message, with each question in full.';

// The list and failed-answer helpers live in questionCardList.ts (the chat
// store's half, in the App chunk); re-exported for the card's users.
export { answerSendFailureAction, applyInteractionUpdate, isOpenCard, keepOpenCards } from './questionCardList';
