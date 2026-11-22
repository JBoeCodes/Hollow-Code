import { describe, it, expect } from 'vitest';
import {
  answerSendFailureAction,
  applyInteractionUpdate,
  keepOpenCards,
  approveAll,
  buildAnswer,
  cardTitle,
  formErrors,
  initialFormState,
  optionOutcome,
  questionReason,
  questionScenario,
  sendsAsMessage,
  submitBlockReason,
} from './questionCards';
import type { ChatInteraction, InteractionRequest } from './types';

it('shows a one-line reason, and prefers reason over help text', () => {
  expect(questionReason({ reason: '  Why this\nchoice ' })).toBe('Why this choice');
  expect(questionReason({ helpText: 'Because\nit matters' })).toBe('Because it matters');
  expect(questionReason({ reason: 'Short', helpText: 'Longer help' })).toBe('Short');
  expect(questionReason({ reason: '   ', helpText: 'Fallback' })).toBe('Fallback');
  expect(questionReason({})).toBe('');
});

it('reads a decision card\'s scenario and outcomes as plain one-line text', () => {
  expect(questionScenario({ scenario: '  You pay $12\nfor 40 seats. ' })).toBe('You pay $12 for 40 seats.');
  expect(questionScenario({ scenario: '   ' })).toBe('');
  expect(questionScenario({})).toBe('');
  expect(optionOutcome({ outcome: ' $384 a year,\n$96 less. ' })).toBe('$384 a year, $96 less.');
  expect(optionOutcome({ outcome: null })).toBe('');
  expect(optionOutcome({})).toBe('');
  // Markup stays text: the card draws it through a text node.
  expect(optionOutcome({ outcome: '<img src=x onerror=1> **bold**' })).toBe('<img src=x onerror=1> **bold**');
});

const layout: InteractionRequest = {
  mode: 'questions',
  supersedeOnUserMessage: true,
  questions: [
    {
      id: 'layout', prompt: 'Which layout?', required: true, answerMode: 'single', allowCustom: true,
      options: [{ id: 'grid', label: 'Grid', recommended: true }, { id: 'list', label: 'List' }],
    },
    { id: 'notes', prompt: 'Anything else?', required: false, answerMode: 'text', options: [], allowCustom: false },
    {
      id: 'count', prompt: 'How many?', required: false, answerMode: 'text', options: [], allowCustom: false,
      textValidation: { inputType: 'integer', minimum: 1, maximum: 5 },
    },
  ],
};

function card(over: Partial<ChatInteraction> = {}): ChatInteraction {
  return {
    id: 'qi-1', sessionKey: 's-20260926-aaaa0001', turnOrd: 1, runId: 'r1', agent: 'claude',
    nativeSessionId: 'native-1', toolUseId: 'toolu-1', source: 'mcp', mode: 'questions',
    request: layout, targetPath: null, supersedeOnUserMessage: true, status: 'pending',
    statusReason: null, supersededBy: null, answer: null, createdAt: '2026-09-26T00:00:00.000Z', resolvedAt: null,
    ...over,
  };
}

describe('question card forms', () => {
  it('starts empty and reports what is missing', () => {
    const state = initialFormState(layout);
    expect(Object.keys(state.answers)).toEqual(['layout', 'notes', 'count']);
    expect(formErrors(layout, state)).toEqual({ layout: 'Required' });
  });

  it('builds only the answered fields', () => {
    const state = initialFormState(layout);
    state.answers.layout.selected = ['grid'];
    expect(formErrors(layout, state)).toEqual({});
    expect(buildAnswer(layout, state)).toEqual({ answers: { layout: { selected: ['grid'] } } });

    state.answers.layout.otherOn = true;
    state.answers.layout.other = '  Masonry ';
    expect(buildAnswer(layout, state)).toEqual({ answers: { layout: { other: 'Masonry' } } });

    state.answers.count.text = '9';
    expect(formErrors(layout, state)).toEqual({ count: 'Out of range' });
    state.answers.count.text = '2.5';
    expect(formErrors(layout, state)).toEqual({ count: 'Enter a whole number' });
    state.answers.count.text = '3';
    state.answers.notes.text = ' ok ';
    expect(buildAnswer(layout, state).answers).toEqual({
      layout: { other: 'Masonry' }, notes: { text: 'ok' }, count: { text: '3' },
    });
  });

  it('validates the other modes', () => {
    const confirm: InteractionRequest = { mode: 'confirmation', supersedeOnUserMessage: true, confirmation: { prompt: 'Go?' } };
    const c = initialFormState(confirm);
    expect(formErrors(confirm, c)).toEqual({ confirmation: 'Choose one' });
    c.accepted = false;
    expect(buildAnswer(confirm, c)).toEqual({ accepted: false });

    const checkbox: InteractionRequest = {
      mode: 'checkbox', supersedeOnUserMessage: true,
      checkbox: { prompt: 'Pick', options: [{ id: 'a', label: 'A' }, { id: 'b', label: 'B' }], defaultSelected: ['a'], maxSelected: 1 },
    };
    const k = initialFormState(checkbox);
    expect(k.selected).toEqual(['a']);
    k.selected = ['a', 'b'];
    expect(formErrors(checkbox, k)).toEqual({ checkbox: 'Select at most 1' });

    const verdicts: InteractionRequest = {
      mode: 'verdicts', supersedeOnUserMessage: true,
      verdicts: { prompt: 'Review', items: [{ id: 'one', label: 'One' }, { id: 'two', label: 'Two' }], verdicts: ['approve', 'reject', 'defer'], requireReasonOn: ['reject'], allowBulkApprove: true },
    };
    let v = initialFormState(verdicts);
    v.verdicts.two = { verdict: 'reject', reason: '' };
    expect(formErrors(verdicts, v)).toEqual({ one: 'Choose a verdict', two: 'Give a reason' });
    v = approveAll(verdicts, v);
    v.verdicts.two.reason = 'risky';
    expect(formErrors(verdicts, v)).toEqual({});
    expect(buildAnswer(verdicts, v)).toEqual({ verdicts: { one: { verdict: 'approve' }, two: { verdict: 'reject', reason: 'risky' } } });
  });

  it('titles cards by mode', () => {
    expect(cardTitle(card())).toBe('Questions from the agent');
    expect(cardTitle(card({ request: { ...layout, title: 'Before I start' } }))).toBe('Before I start');
    expect(cardTitle(card({ request: { mode: 'confirmation', supersedeOnUserMessage: true, confirmation: { prompt: 'x' } } }))).toBe('Confirm');
  });
});

describe('submitting and send failures', () => {
  it('blocks Submit while busy, loading or stale, never for the session that asked', () => {
    const base = { busy: false, loading: false };
    expect(submitBlockReason({ card: card(), ...base })).toBeNull();
    expect(submitBlockReason({ card: card(), ...base, busy: true })).toMatch(/current turn/);
    expect(submitBlockReason({ card: card(), ...base, loading: true })).toMatch(/loading/);
    expect(submitBlockReason({ card: card({ status: 'stale' }), ...base })).toMatch(/changed/);
    // 2026-11-04: a Main check-in's card, after the next turn started a fresh
    // native session, kept Send disabled. Another session, or none, sends.
    expect(submitBlockReason({ card: card({ nativeSessionId: 'an-older-session' }), ...base })).toBeNull();
    expect(submitBlockReason({ card: card({ nativeSessionId: null }), ...base })).toBeNull();
  });

  it('sends as a message whenever the conversation no longer resumes the asking session', () => {
    expect(sendsAsMessage(card({ nativeSessionId: 'native-1' }), 'native-1')).toBe(false);
    expect(sendsAsMessage(card({ nativeSessionId: 'native-1' }), 'native-2')).toBe(true);
    expect(sendsAsMessage(card({ nativeSessionId: 'native-1' }), null)).toBe(true);
    expect(sendsAsMessage(card({ nativeSessionId: null }), null)).toBe(true);
  });

  it('restores a reopened card, whatever session asked, and the answers only once', () => {
    // 2026-11-04: a card can always be sent now, so a reopened one comes back
    // on its own; the composer gets the answers only when it didn't reopen.
    expect(answerSendFailureAction({ reopenOk: true, sameChat: true })).toBe('restoreCard');
    expect(answerSendFailureAction({ reopenOk: false, sameChat: true })).toBe('toComposer');
    // The chat was reset to another conversation meanwhile (a recovered
    // working folder): the card can't show there, so the composer has them.
    expect(answerSendFailureAction({ reopenOk: true, sameChat: false })).toBe('toComposer');
  });

  it('keeps only the current conversation’s open cards', () => {
    const one = card();
    const list = applyInteractionUpdate([], one, one.sessionKey);
    expect(list).toEqual([one]);
    expect(applyInteractionUpdate(list, card({ id: 'qi-2', sessionKey: 's-20260926-bbbb0002' }), one.sessionKey)).toBe(list);
    const stale = card({ status: 'stale' });
    expect(applyInteractionUpdate(list, stale, one.sessionKey)).toEqual([stale]);
    expect(applyInteractionUpdate(list, card({ status: 'answered' }), one.sessionKey)).toEqual([]);
    expect(applyInteractionUpdate(list, card({ status: 'superseded' }), null)).toBe(list);
  });

  it('a finished turn keeps an open card and drops a closed one', () => {
    const open = card();
    const list = [open];
    expect(keepOpenCards(list)).toBe(list);
    const closed = card({ id: 'qi-closed', status: 'answered' });
    expect(keepOpenCards([open, closed])).toEqual([open]);
    expect(keepOpenCards([closed])).toEqual([]);
  });
});
