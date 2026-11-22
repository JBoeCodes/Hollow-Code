import { describe, expect, it } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { GRACE_MS, answerWords, createQuestionDock, dockKey, dockRows, questionCount, startCursor, stepError, tooSoon, waitingWords } from './questionDock';
import { MOVED_STATUS, NOT_SENT_STATUS, QUEUE_LABEL, QUEUED_STATUS, askRefusalStatus, askSendLabel } from './canvas/askSend';
import { answeredRevision, answersBack, askHold, holdKey, setAskHold, settleHeldAnswers } from './canvas/askHold';
import type { ChatInteraction, InteractionQuestion } from './types';

const q = (over: Partial<InteractionQuestion> = {}): InteractionQuestion => ({
  id: 'install', prompt: 'Install main 4f196bf4?', required: true, answerMode: 'single', allowCustom: true,
  options: [{ id: 'do', label: 'Do it' }, { id: 'later', label: 'Not now', recommended: true }, { id: 'never', label: 'Never' }],
  ...over,
});
const blank = { selected: [], otherOn: false, other: '', text: '' };

describe('docked question cards (2026-10-14)', () => {
  it('shows a choice question\'s options, then "Other…", and starts on the recommended one', () => {
    expect(dockRows(q()).map(r => r.kind === 'option' ? r.label : `[${r.label}]`)).toEqual(['Do it', 'Not now', 'Never', '[Other…]']);
    expect(dockRows(q({ allowCustom: false, customLabel: 'x' })).map(r => r.kind)).toEqual(['option', 'option', 'option']);
    expect(dockRows(q({ customLabel: 'Something else' })).at(-1)).toEqual({ kind: 'other', label: 'Something else' });
    expect(dockRows(q({ answerMode: 'text', options: [] }))).toEqual([]);
    expect(startCursor(q())).toBe(1);
    expect(startCursor(q({ options: [{ id: 'a', label: 'A' }] }))).toBe(0);
  });

  it('carries each option\'s outcome as one plain line, or null', () => {
    const rows = dockRows(q({ options: [
      { id: 'a', label: 'Annual', outcome: ' $384 a year,\n$96 less. ', recommended: true },
      { id: 'b', label: 'Monthly', outcome: '  ' },
      { id: 'c', label: 'Never' },
    ] }));
    expect(rows.map(r => r.kind === 'option' ? r.outcome : 'other')).toEqual(['$384 a year, $96 less.', null, null, 'other']);
    expect(startCursor(q({ options: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B', outcome: 'x', recommended: true }] }))).toBe(1);
  });

  it('maps plain keys only: arrows move and wrap, digits and Enter choose, Space ticks, ← is Back, Esc hides', () => {
    const single = { mode: 'single' as const, rows: 4, cursor: 1 };
    expect(dockKey({ key: 'ArrowDown' }, single)).toEqual({ type: 'move', to: 2 });
    expect(dockKey({ key: 'ArrowUp' }, { ...single, cursor: 0 })).toEqual({ type: 'move', to: 3 });
    expect(dockKey({ key: 'ArrowDown' }, { ...single, cursor: 3 })).toEqual({ type: 'move', to: 0 });
    expect(dockKey({ key: 'End' }, single)).toEqual({ type: 'move', to: 3 });
    expect(dockKey({ key: 'Enter' }, single)).toEqual({ type: 'pick', index: 1 });
    expect(dockKey({ key: '3' }, single)).toEqual({ type: 'pick', index: 2 });
    expect(dockKey({ key: '5' }, single)).toBeNull();
    expect(dockKey({ key: ' ' }, single)).toEqual({ type: 'pick', index: 1 });
    expect(dockKey({ key: 'ArrowLeft' }, single)).toEqual({ type: 'back' });
    expect(dockKey({ key: 'Escape' }, single)).toEqual({ type: 'hide' });
    // A multi-choice question ticks with Space and digits; Enter goes on.
    expect(dockKey({ key: 'Enter' }, { ...single, mode: 'multi' })).toEqual({ type: 'confirm' });
    expect(dockKey({ key: '2' }, { ...single, mode: 'multi' })).toEqual({ type: 'pick', index: 1 });
    // Never with ⌘, ⌃ or ⌥ (macOS owns those), nor while composing text.
    for (const mod of ['metaKey', 'ctrlKey', 'altKey', 'isComposing'] as const) {
      expect(dockKey({ key: '1', [mod]: true }, single)).toBeNull();
      expect(dockKey({ key: 'Enter', [mod]: true }, single)).toBeNull();
    }
    expect(dockKey({ key: 'Enter', shiftKey: true }, single)).toBeNull();
    expect(dockKey({ key: 'a' }, single)).toBeNull();
  });

  it('a step goes on only with a valid answer, and says what was chosen', () => {
    expect(stepError(q(), blank)).toBe('Required');
    expect(stepError(q(), { ...blank, selected: ['do'] })).toBeNull();
    expect(stepError(q(), { ...blank, otherOn: true })).toBe('Type your answer');
    expect(stepError(q(), { ...blank, otherOn: true, other: 'Tomorrow' })).toBeNull();
    expect(stepError(q({ required: false }), blank)).toBeNull();
    expect(stepError(q({ answerMode: 'text', options: [] }), { ...blank, text: '  ' })).toBe('Required');
    expect(answerWords(q(), { ...blank, selected: ['later'] })).toBe('Not now');
    expect(answerWords(q({ answerMode: 'multi' }), { ...blank, selected: ['do', 'never'], otherOn: true, other: 'Ask Jordan' })).toBe('Do it, Never, Ask Jordan');
    expect(answerWords(q(), undefined)).toBe('No answer');
  });

  it('counts the questions waiting for the chip', () => {
    const card = (request: ChatInteraction['request']) => ({ request });
    expect(questionCount([card({ mode: 'questions', supersedeOnUserMessage: true, questions: [q(), q(), q()] }), card({ mode: 'confirmation', supersedeOnUserMessage: true })])).toBe(4);
    expect(waitingWords(1)).toBe('1 question waiting');
    expect(waitingWords(3)).toBe('3 questions waiting');
  });
});

describe('which card the dock shows (review fixes)', () => {
  const card = (id: string, status: ChatInteraction['status']) => ({ id, status, request: { mode: 'questions', supersedeOnUserMessage: true, questions: [q()] } }) as unknown as ChatInteraction;
  it('opens a pending card, keeps a hidden one behind the chip, and an out-of-date one closed until asked for', () => createRoot(dispose => {
    const [cards, setCards] = createSignal<ChatInteraction[]>([card('a', 'pending')]);
    const dock = createQuestionDock(cards);
    expect([dock.open(), dock.cardId()]).toEqual([true, 'a']);
    // The same pending card, refreshed, stays open (a finished turn must not close it).
    setCards([card('a', 'pending')]);
    expect([dock.open(), dock.cardId()]).toEqual([true, 'a']);
    dock.hide();
    expect(dock.open()).toBe(false);
    // A fresh copy of the same card stays hidden; a new card opens.
    setCards([card('a', 'pending')]);
    expect(dock.open()).toBe(false);
    setCards([card('b', 'pending')]);
    expect(dock.open()).toBe(true);
    // Typing when it came: it waits behind the chip.
    dock.hideFor('b');
    expect(dock.open()).toBe(false);
    dock.show();
    expect(dock.open()).toBe(true);
    // Out of date: closed, so the message box stays, until the chip opens it.
    setCards([card('c', 'stale')]);
    expect([dock.open(), dock.cardId()]).toEqual([false, 'c']);
    dock.show();
    expect(dock.open()).toBe(true);
    dispose();
  }));
});

describe('ask-canvas Send (2026-10-14)', () => {
  it('names Hollow\'s Send button from the form, as plain text', () => {
    expect(askSendLabel('<hc-ask title="T" send="Use these answers">')).toBe('Use these answers');
    expect(askSendLabel("<hc-ask send='Go &amp; send'>")).toBe('Go & send');
    expect(askSendLabel('<hc-ask title="No send">')).toBe('Send answers');
    expect(askSendLabel('<p send="x"></p>')).toBe('Send answers');
    expect([...askSendLabel(`<hc-ask send="${'y'.repeat(80)}">`)]).toHaveLength(40);
  });

  it('says in plain words why a Send didn\'t go, with Try again where it can', () => {
    expect(askRefusalStatus('busy')).toMatchObject({ tone: 'warning', retry: true });
    expect(askRefusalStatus('locked').text).toBe('Already answered.');
    expect(askRefusalStatus('not-current').retry).toBeUndefined();
    expect(askRefusalStatus('unrequested')).toMatchObject({ tone: 'danger', retry: true });
    for (const r of ['invalid', 'not-ask', 'locked', 'not-current', 'unavailable', 'unrequested', 'busy', 'not-sent', 'send-failed'] as const) {
      expect(askRefusalStatus(r).text).not.toMatch(/copy|clipboard/i);
    }
  });

  it('says a failed send kept the answers in the canvas, with Send now and Take them back, never the message box (P1-33)', () => {
    const notSent = askRefusalStatus('not-sent');
    expect(notSent).toBe(NOT_SENT_STATUS);
    expect(notSent.text).toBe('Your answers weren’t sent. Send them now, or take them back to change them.');
    expect(notSent.text).not.toMatch(/message box/);
    // Send now is the callout's own button: no Try again beside it.
    expect(notSent).toMatchObject({ tone: 'warning' });
    expect(notSent.retry).toBeUndefined();
    expect(askRefusalStatus('send-failed')).toEqual({ text: 'Your answers weren’t sent. Try again.', tone: 'warning', retry: true });
    for (const r of ['not-sent', 'send-failed'] as const) expect(askRefusalStatus(r).text).not.toBe(askRefusalStatus('unavailable').text);
    expect(MOVED_STATUS.text).toMatch(/nothing was sent/);
  });

  it('queues while the agent answers instead of asking to send again (P1-33)', () => {
    expect(QUEUE_LABEL).toBe('Queue answers');
    expect(QUEUED_STATUS).toEqual({ text: 'Queued. Your answers go as soon as this turn ends.', tone: 'info' });
    // Only a conversation still opening refuses as busy now.
    expect(askRefusalStatus('busy').text).not.toMatch(/still answering|send again when it finishes/i);
  });

  it('keeps one hold per conversation and revision, shared by every copy; a held send that goes clears it, one that fails becomes not-sent (P1-33)', () => {
    const key = holdKey('s-1', 'c@1');
    expect(holdKey('s-2', 'c@1')).not.toBe(key);
    expect(askHold(key)).toBeNull();
    setAskHold(key, { kind: 'queued', held: 'h-1' });
    expect(askHold(key)).toEqual({ kind: 'queued', held: 'h-1' });
    expect(settleHeldAnswers('h-other', 'x')).toBe(false);
    expect(settleHeldAnswers('h-1', 'the answers')).toBe(true);
    expect(askHold(key)).toEqual({ kind: 'not-sent', text: 'the answers' });
    setAskHold(key, { kind: 'queued', held: 'h-2' });
    expect(settleHeldAnswers('h-2', null)).toBe(true);
    expect(askHold(key)).toBeNull();
  });

  it('sends an answers message that came back unsent to its canvas, never the box; other text is not one (P1-33)', () => {
    const text = 'Answered canvas "c":\n- Q: A\n\n```hollow-canvas-answer\n{"canvasId":"c","revision":2,"answers":{"q":"A"}}\n```';
    expect(answeredRevision(text)).toBe('c@2');
    expect(answeredRevision('Fix the parser')).toBeNull();
    expect(answeredRevision('```hollow-canvas-answer\n{"canvasId":3}\n```')).toBeNull();
    expect(answersBack('s-9', 'Fix the parser')).toBe(false);
    expect(answersBack('s-9', text)).toBe(true);
    expect(askHold(holdKey('s-9', 'c@2'))).toEqual({ kind: 'not-sent', text });
  });
});

describe('the half-second grace (2026-10-15)', () => {
  it('refuses an answering key or click less than half a second after something moved', () => {
    expect(GRACE_MS).toBe(500);
    // The dock showed (or the frame changed height) at 1000.
    expect(tooSoon(1000, 1000)).toBe(true);
    expect(tooSoon(1000, 1499)).toBe(true);
    expect(tooSoon(1000, 1500)).toBe(false);
    expect(tooSoon(1000, 2600)).toBe(false);
    // Nothing moved yet: nothing to wait out.
    expect(tooSoon(null, 0)).toBe(false);
    expect(tooSoon(1000, 1100, 50)).toBe(false);
  });
});
