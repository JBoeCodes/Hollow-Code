import { describe, expect, it } from 'vitest';
import { CONTINUE_PROMPT, canContinueChat, configResetClause, regenerateIndex, retryIndex, sessionResetNotice, stopOffersContinue, type ContinueTurn, type RetryTurn } from './chatContinue';

const idle = { busy: false, queued: false, sessionId: 'sess-1' };
const user: ContinueTurn = { role: 'user' };
const done: ContinueTurn = { role: 'assistant', status: 'success' };

describe('manual Continue (Paperclip PR 14b)', () => {
  it('sends exactly the interrupted-run prompt', () => {
    expect(CONTINUE_PROMPT).toBe('Your previous run was interrupted. Continue from where you left off.');
  });

  it('is offered after a live stop and after a cancelled, interrupted or stalled turn', () => {
    expect(canContinueChat([user, { role: 'assistant' }, { role: 'system', stopNotice: true }], idle)).toBe(true);
    expect(canContinueChat([user, { role: 'system', stopNotice: true }], idle)).toBe(true);
    expect(canContinueChat([user, { role: 'assistant', status: 'cancelled' }], idle)).toBe(true);
    expect(canContinueChat([user, { role: 'assistant', status: 'interrupted' }], idle)).toBe(true);
    expect(canContinueChat([user, { role: 'assistant', status: 'error', resultKind: 'stalled' }], idle)).toBe(true);
  });

  it('skips restored-history notices that describe earlier turns', () => {
    const restored: ContinueTurn[] = [
      user, { role: 'assistant', status: 'error' },
      user, { role: 'assistant', status: 'cancelled' },
      { role: 'system', hydratedNotice: true },
      { role: 'system', hydratedNotice: true },
    ];
    expect(canContinueChat(restored, idle)).toBe(true);
    // A restored conversation whose last turn succeeded offers nothing.
    expect(canContinueChat([user, { role: 'assistant', status: 'cancelled' }, user, done, { role: 'system', hydratedNotice: true }], idle)).toBe(false);
  });

  it('looks past the notice that a queued message was put back (plan completion L5)', () => {
    const restoredQueue: ContinueTurn[] = [user, { role: 'assistant', status: 'interrupted' }, { role: 'system', aboutEarlierTurn: true }];
    expect(canContinueChat(restoredQueue, idle)).toBe(true);
    // An ordinary system notice still ends the offer.
    expect(canContinueChat([user, { role: 'assistant', status: 'interrupted' }, { role: 'system' }], idle)).toBe(false);
  });

  it('is not offered while busy, queued, without a session, after a queued cancel, a later user turn or a success', () => {
    const stopped: ContinueTurn[] = [user, { role: 'system', stopNotice: true }];
    expect(canContinueChat(stopped, { ...idle, busy: true })).toBe(false);
    expect(canContinueChat(stopped, { ...idle, queued: true })).toBe(false);
    expect(canContinueChat(stopped, { ...idle, sessionId: null })).toBe(false);
    // A queued cancel (the turn never started) is a system notice without stopNotice.
    expect(canContinueChat([user, { role: 'system' }], idle)).toBe(false);
    expect(canContinueChat([...stopped, user], idle)).toBe(false);
    expect(canContinueChat([user, done], idle)).toBe(false);
    expect(canContinueChat([user, { role: 'assistant', status: 'error', resultKind: 'providerQuota' }], idle)).toBe(false);
    expect(canContinueChat([], idle)).toBe(false);
  });

  it('is not offered for a stop before the agent was spawned, live or restored', () => {
    // The live notice of a before-spawn stop carries no stopNotice.
    expect(stopOffersContinue({ beforeSpawn: true })).toBe(false);
    expect(stopOffersContinue({})).toBe(true);
    expect(stopOffersContinue({ beforeSpawn: false })).toBe(true);
    expect(canContinueChat([user, { role: 'system', stopNotice: stopOffersContinue({ beforeSpawn: true }) }], idle)).toBe(false);
    // A restored turn that was stopped before spawn, even with status cancelled.
    expect(canContinueChat([user, { role: 'assistant', status: 'cancelled', stoppedBeforeSpawn: true }], idle)).toBe(false);
    expect(canContinueChat([user, { role: 'assistant', status: 'cancelled', stoppedBeforeSpawn: false }], idle)).toBe(true);
  });

  it('announces a fresh native session neutrally, naming the agent', () => {
    expect(sessionResetNotice('resumeFailed', 'Claude')).toBe(
      'The previous Claude session could not be resumed, so Hollow started a fresh one with the recent conversation.',
    );
    expect(sessionResetNotice('configChanged', 'Codex')).toBe(
      "This conversation's agent settings changed, so Hollow started a fresh Codex session with the recent conversation.",
    );
    expect(sessionResetNotice('moved', 'Claude')).toBe(
      'This conversation moved to another workspace, so Hollow started a fresh Claude session there with the recent conversation.',
    );
    // Review finding A·R10: a refused resume id is said out loud, never silent.
    expect(sessionResetNotice('invalidId', 'Grok')).toBe(
      'Hollow couldn’t safely pick up the saved Grok conversation, so it started a fresh one with the recent conversation.',
    );
    for (const reason of ['resumeFailed', 'configChanged', 'moved', 'invalidId']) {
      expect(sessionResetNotice(reason, 'Claude')).not.toMatch(/[—<>]/);
    }
  });

  // OF03: a config-changed reset says what changed (the event's `changed`,
  // OB03's `chat_resume::PARTS`), in plain words.
  it('names each changed part of a config-changed reset in plain words', () => {
    const fresh = ', so Hollow started a fresh Claude session with the recent conversation.';
    const cases: Array<[string, string]> = [
      ['contract', "Hollow's guidance for the agent was updated"],
      ['agent', 'The agent changed'],
      ['model', 'The model changed'],
      ['mode', 'The permission mode changed'],
      ['cwd', 'The working folder changed'],
      ['mcp', "Hollow's tools for the agent changed"],
      ['caps', "Hollow's tools for the agent changed"],
      ['canvases', 'The Canvases setting changed'],
      ['workspace', 'The kind of conversation changed'],
      ['skills', 'The skills changed'],
    ];
    for (const [part, clause] of cases) {
      const notice = sessionResetNotice('configChanged', 'Claude', [part]);
      expect(notice).toBe(clause + fresh);
      expect(notice).not.toMatch(/[—<>]/);
    }
  });

  it('joins several changed parts naturally, in a fixed order', () => {
    expect(sessionResetNotice('configChanged', 'Codex', ['mode', 'model'])).toBe(
      'The model and the permission mode changed, so Hollow started a fresh Codex session with the recent conversation.',
    );
    expect(sessionResetNotice('configChanged', 'Claude', ['canvases', 'cwd', 'model'])).toBe(
      'The model, the working folder and the Canvases setting changed, so Hollow started a fresh Claude session with the recent conversation.',
    );
    expect(sessionResetNotice('configChanged', 'Claude', ['contract', 'mcp'])).toBe(
      "Hollow's tools for the agent changed and Hollow's guidance for the agent was updated, so Hollow started a fresh Claude session with the recent conversation.",
    );
    expect(sessionResetNotice('configChanged', 'Claude', ['workspace', 'cwd'])).toBe(
      'The working folder and the kind of conversation changed, so Hollow started a fresh Claude session with the recent conversation.',
    );
    // A repeated or unrecognised name beside a known one is left out.
    expect(sessionResetNotice('configChanged', 'Claude', ['model', 'model', 'unknown', 'shiny'])).toBe(
      'The model changed, so Hollow started a fresh Claude session with the recent conversation.',
    );
  });

  it('says the tools once for mcp and caps, and leaves caps out beside an agent change', () => {
    const fresh = ', so Hollow started a fresh Claude session with the recent conversation.';
    expect(sessionResetNotice('configChanged', 'Claude', ['mcp', 'caps'])).toBe("Hollow's tools for the agent changed" + fresh);
    expect(sessionResetNotice('configChanged', 'Claude', ['caps', 'model'])).toBe(
      "The model and Hollow's tools for the agent changed" + fresh,
    );
    // An agent change always changes caps: only the agent is named.
    expect(sessionResetNotice('configChanged', 'Claude', ['agent', 'caps'])).toBe('The agent changed' + fresh);
    expect(sessionResetNotice('configChanged', 'Claude', ['caps', 'agent', 'model'])).toBe('The agent and the model changed' + fresh);
    // ...but an mcp change beside it still is.
    expect(sessionResetNotice('configChanged', 'Claude', ['agent', 'caps', 'mcp'])).toBe(
      "The agent and Hollow's tools for the agent changed" + fresh,
    );
  });

  it('gives the drawer the same clause, and none for other reasons', () => {
    expect(configResetClause('configChanged', ['mode', 'model'])).toBe('The model and the permission mode changed');
    expect(configResetClause('configChanged', ['contract'])).toBe("Hollow's guidance for the agent was updated");
    expect(configResetClause('configChanged', ['unknown'])).toBeNull();
    expect(configResetClause('configChanged', [])).toBeNull();
    expect(configResetClause('configChanged', null)).toBeNull();
    expect(configResetClause('resumeFailed', ['model'])).toBeNull();
    expect(configResetClause(undefined, ['model'])).toBeNull();
  });

  it("keeps today's wording when the reset names nothing it knows", () => {
    const today = "This conversation's agent settings changed, so Hollow started a fresh Claude session with the recent conversation.";
    expect(sessionResetNotice('configChanged', 'Claude')).toBe(today);
    expect(sessionResetNotice('configChanged', 'Claude', null)).toBe(today);
    expect(sessionResetNotice('configChanged', 'Claude', [])).toBe(today);
    expect(sessionResetNotice('configChanged', 'Claude', ['unknown'])).toBe(today);
    expect(sessionResetNotice('configChanged', 'Claude', ['shiny'])).toBe(today);
    // Only a config-changed reset's wording changes.
    expect(sessionResetNotice('resumeFailed', 'Claude', ['model'])).toBe(
      'The previous Claude session could not be resumed, so Hollow started a fresh one with the recent conversation.',
    );
    expect(sessionResetNotice('moved', 'Claude', ['cwd'])).toBe(
      'This conversation moved to another workspace, so Hollow started a fresh Claude session there with the recent conversation.',
    );
  });
});

describe('Retry under a failed last turn (P1-27)', () => {
  const ask: RetryTurn = { role: 'user', text: 'Draft the release notes.' };
  const failed: RetryTurn = { role: 'assistant', text: '(error: spawn failed)', status: 'error' };
  const free = { busy: false, queued: false };

  it('offers the message that failed, live or restored', () => {
    expect(retryIndex([ask, failed], free)).toBe(0);
    expect(retryIndex([ask, { role: 'assistant', text: '', status: 'failed' }], free)).toBe(0);
    expect(retryIndex([ask, { role: 'assistant', text: 'Partly done', receipt: { runStatus: 'error' } }], free)).toBe(0);
    // A tool step before the failure, then a restored error notice after it.
    expect(retryIndex([user, done, ask, { role: 'assistant', text: 'Reading.' }, failed, { role: 'system', text: 'usage limit', hydratedNotice: true }], free)).toBe(2);
  });

  it('is not offered while busy or queued, after a success, a stop or a stall, or a later message', () => {
    expect(retryIndex([ask, failed], { ...free, busy: true })).toBe(-1);
    expect(retryIndex([ask, failed], { ...free, queued: true })).toBe(-1);
    expect(retryIndex([ask, { role: 'assistant', text: 'Done.', status: 'success' }], free)).toBe(-1);
    expect(retryIndex([ask, { role: 'assistant', text: '', status: 'cancelled' }], free)).toBe(-1);
    expect(retryIndex([ask, { role: 'assistant', text: '', status: 'error', resultKind: 'stalled' }], free)).toBe(-1);
    expect(retryIndex([ask, failed, { role: 'user', text: 'Try again' }], free)).toBe(-1);
    expect(retryIndex([ask, failed, { role: 'system', text: 'Switched account', notice: 'accountSwitch' }], free)).toBe(-1);
    // Nothing to send again: an empty message (an image only) or none at all.
    expect(retryIndex([{ role: 'user', text: '  ' }, failed], free)).toBe(-1);
    expect(retryIndex([failed], free)).toBe(-1);
  });

  it('reads the run’s own status, not the receipt’s verdict (review 1)', () => {
    // A stop's receipt and a success whose checks failed both say "failed".
    const verdict = (runStatus: string) => ({ runStatus, status: 'failed' }) as RetryTurn['receipt'];
    expect(retryIndex([ask, { role: 'assistant', text: '', status: 'cancelled', receipt: verdict('cancelled') }], free)).toBe(-1);
    expect(retryIndex([ask, { role: 'assistant', text: 'Edited; tests fail.', status: 'success', receipt: verdict('success') }], free)).toBe(-1);
  });

  it('leaves a refusal alone, live or restored', () => {
    expect(retryIndex([ask, { role: 'assistant', text: '', status: 'error', resultKind: 'refusal' }, { role: 'system', text: 'declined', hydratedNotice: true }], free)).toBe(-1);
  });

  it('never resends a message Hollow wrote (a COO wake)', () => {
    const wake = { role: 'user' as const, text: 'wake' };
    expect(retryIndex([wake, failed], free, t => t === wake)).toBe(-1);
    expect(retryIndex([ask, failed], free, t => t === wake)).toBe(0);
  });
});

describe('Retry under a finished last reply (overnight 2026-11-04 C3)', () => {
  const ask: RetryTurn = { role: 'user', text: 'Draft the release notes.' };
  const finished: RetryTurn = { role: 'assistant', text: 'Here are the notes.', status: 'success' };
  const free = { busy: false, queued: false };

  it('offers the reply footer and the message it answered, live or restored', () => {
    expect(regenerateIndex([ask, finished], free)).toEqual({ reply: 1, ask: 0 });
    // A tool step, then the answer, then a restored notice about an earlier turn.
    expect(regenerateIndex([user, done, ask, { role: 'assistant', text: 'Reading.' }, finished, { role: 'system', text: 'Some older transcript content is unavailable.', hydratedNotice: true }], free)).toEqual({ reply: 4, ask: 2 });
    // A restored reply with no stored status reads its receipt's run status.
    expect(regenerateIndex([ask, { role: 'assistant', text: 'Done.', receipt: { runStatus: 'success' } }], free)).toEqual({ reply: 1, ask: 0 });
  });

  it('is not offered while busy or queued, after a failure, a stop or a stall, or on an older reply', () => {
    expect(regenerateIndex([ask, finished], { ...free, busy: true })).toBeNull();
    expect(regenerateIndex([ask, finished], { ...free, queued: true })).toBeNull();
    // A failure has Retry under it; a stop, interruption or stall has Continue.
    expect(regenerateIndex([ask, { role: 'assistant', text: '(error: spawn failed)', status: 'error' }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: 'Done.', status: 'success', receipt: { runStatus: 'error' } }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: '', status: 'cancelled' }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: '', status: 'interrupted' }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: 'Checking.', status: 'success', resultKind: 'stalled' }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: 'Still going.' }], free)).toBeNull();
    // A later message, or a live notice after the reply, leaves it older.
    expect(regenerateIndex([ask, finished, { role: 'user', text: 'Thanks' }], free)).toBeNull();
    expect(regenerateIndex([ask, finished, { role: 'system', text: 'Stopped.', stopNotice: true }], free)).toBeNull();
  });

  it('leaves a refusal alone, as Retry under a failure does', () => {
    expect(regenerateIndex([ask, { role: 'assistant', text: 'I can’t help with that.', status: 'success', resultKind: 'refusal' }], free)).toBeNull();
    expect(regenerateIndex([ask, { role: 'assistant', text: '', status: 'success', resultKind: 'refusal' }, { role: 'system', text: 'declined', hydratedNotice: true }], free)).toBeNull();
  });

  it('never resends a message Hollow wrote or one with no words', () => {
    const wake = { role: 'user' as const, text: 'wake' };
    expect(regenerateIndex([wake, finished], free, t => t === wake)).toBeNull();
    expect(regenerateIndex([{ role: 'user', text: '  ' }, finished], free)).toBeNull();
    expect(regenerateIndex([finished], free)).toBeNull();
  });

  it('never overlaps Retry under a failure or Continue', () => {
    const shapes: RetryTurn[] = [finished, { role: 'assistant', text: '', status: 'error' }, { role: 'assistant', text: '', status: 'cancelled' }, { role: 'assistant', text: '', status: 'success', resultKind: 'stalled' }];
    for (const last of shapes) {
      const offers = [regenerateIndex([ask, last], free) !== null, retryIndex([ask, last], free) >= 0, canContinueChat([ask, last], { ...free, sessionId: 's' })];
      expect(offers.filter(Boolean).length).toBeLessThanOrEqual(1);
    }
  });
});

describe('a rested reset (overnight 9 A9)', () => {
  it('says how long it was away and at what size, or the plain reason without figures', () => {
    expect(sessionResetNotice('rested', 'Claude', null, 'away 3 h at 714K')).toBe(
      'Started fresh: away 3 h at 714K. Hollow gave a new Claude session the recent conversation and its task state instead of re-reading it all.',
    );
    expect(sessionResetNotice('rested', 'Claude')).toBe(
      'Started fresh after more than an hour away. Hollow gave a new Claude session the recent conversation and its task state instead of re-reading it all.',
    );
  });
});
