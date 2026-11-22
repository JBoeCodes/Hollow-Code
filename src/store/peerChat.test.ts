import { describe, expect, it, vi } from 'vitest';
vi.mock('../api', () => ({ invoke: vi.fn() }));
vi.mock('../nativeEvents', () => ({ onNative: vi.fn(() => () => {}) }));
import type { PeerChatDetail, PeerChatEvent, RemoteSession } from '../generated/ipc';
import { buildAnswer, initialFormState } from '../questionCards';
import { invoke } from '../api';
import {
  STREAM_CALL_TIMEOUT_MS, answerRemote, closeRemoteChat, openRemoteChat, remoteChat, sendRemote,
  cardAnswerToWire, emptyRemoteChat, foldPeerChat, openCardOf, readTurns, wireQuestionsToCard, withEarlier,
  type RemoteChat, type WireQuestions,
} from './peerChat';

const M = 'm-8d21c4be90f17a3e';
const KEY = 's-20261116-0000beef';
const home = { machineId: M, slug: 'build-mac', name: 'Mac mini' };
const session: RemoteSession = {
  key: KEY, title: 'Release', agent: 'claude', workspace: null, createdAt: '', lastActivityAt: '', turnCount: 1,
  state: 'running', needsAnswer: false, preview: null, runsOn: null, movedTo: null, movedFrom: null,
};
const ev = (kind: string, data: unknown, key: string | null = KEY, machineId = M): PeerChatEvent => ({ machineId, key, kind, data });
const turn = (ord: number, role: 'user' | 'assistant', extra: Record<string, unknown> = {}) => ({
  id: `${KEY}-${ord}-${role}`, ord, role, text: `${role} ${ord}`, agent: 'claude', startedAt: '2026-11-16T10:00:00Z',
  status: role === 'assistant' ? 'success' : null, error: null, truncated: false, tools: [], questions: null, ...extra,
});
const detail = (ords: number[], live: unknown = null, hasMore = false): PeerChatDetail => ({
  home,
  detail: { session, permissionMode: 'acceptEdits', turns: ords.flatMap(o => [turn(o, 'user'), turn(o, 'assistant')]), hasMore, live: live as never },
});
const live = (ord: number, gen: number, text: string) => ({ key: KEY, ord, gen, startedAt: '', text, truncated: false, tools: [] });

function opened(): RemoteChat {
  return foldPeerChat(emptyRemoteChat(M, KEY), ev('detail', detail([1, 2], live(3, 1, 'Drafting'), true)));
}

describe('the live fold (the phone contract\'s client rules)', () => {
  it('appends only to the live view whose ord and gen it names', () => {
    let chat = opened();
    expect(chat.live?.text).toBe('Drafting');
    chat = foldPeerChat(chat, ev('append', { key: KEY, ord: 3, gen: 1, text: ' the notes.' }));
    expect(chat.live?.text).toBe('Drafting the notes.');
    // Another ord or gen: dropped (Rust fetches the chat again; a detail follows).
    for (const stale of [{ ord: 2, gen: 1 }, { ord: 3, gen: 2 }]) {
      expect(foldPeerChat(chat, ev('append', { key: KEY, ...stale, text: 'x' }))).toBe(chat);
    }
    // No live view held: nothing to add to.
    const none = foldPeerChat(chat, ev('live', null));
    expect(none.live).toBeNull();
    expect(foldPeerChat(none, ev('append', { key: KEY, ord: 3, gen: 1, text: 'x' }))).toBe(none);
  });

  it('a live frame replaces the view, a turn drops it, and the detail after brings the answer', () => {
    let chat = opened();
    chat = foldPeerChat(chat, ev('live', { ...live(3, 1, 'Drafting the notes. Done.'), truncated: true, tools: [{ id: 't', name: 'Write', status: 'done', detail: null }] }));
    expect(chat.live).toMatchObject({ text: 'Drafting the notes. Done.', truncated: true });
    expect(chat.live?.tools).toHaveLength(1);
    chat = foldPeerChat(chat, ev('turn', { key: KEY, ord: 3, status: 'success' }));
    expect(chat.live).toBeNull();
    chat = foldPeerChat(chat, ev('detail', detail([2, 3])));
    expect(chat.turns.map(t => t.ord)).toEqual([1, 1, 2, 2, 3, 3]);
    expect(chat.hasMore).toBe(true);
  });

  it('state, connection and the end of a stream; events for other chats are ignored', () => {
    let chat = opened();
    chat = foldPeerChat(chat, ev('state', { key: KEY, state: 'idle', needsAnswer: true }));
    expect(chat.session).toMatchObject({ state: 'idle', needsAnswer: true });
    chat = foldPeerChat(chat, ev('connected', { reconnected: false }));
    expect(chat.connected).toBe(true);
    chat = foldPeerChat(chat, ev('disconnected', { message: "Can't reach Mac mini.", retryInMs: 1000 }));
    expect(chat).toMatchObject({ connected: false, offline: "Can't reach Mac mini." });
    chat = foldPeerChat(chat, ev('connected', { reconnected: true }));
    expect(chat.offline).toBeNull();
    const failure = { code: 'pairing-needed', message: 'Pair it again.', state: 'pairing-needed', attemptsLeft: null };
    chat = foldPeerChat(chat, ev('ended', failure));
    expect(chat.ended).toEqual(failure);
    expect(foldPeerChat(chat, ev('live', null, 's-other'))).toBe(chat);
    expect(foldPeerChat(chat, ev('live', null, KEY, 'm-0000000000000001'))).toBe(chat);
    expect(foldPeerChat(chat, ev('live', null, null))).toBe(chat);
    expect(foldPeerChat(chat, ev('sessions', { keys: [KEY] }))).toBe(chat);
  });

  it('an earlier page goes in front once; turns another Mac sent are read leniently', () => {
    let chat = foldPeerChat(emptyRemoteChat(M, KEY), ev('detail', detail([3, 4], null, true)));
    chat = withEarlier(chat, detail([1, 2, 3], null, false));
    expect(chat.turns.map(t => t.ord)).toEqual([1, 1, 2, 2, 3, 3, 4, 4]);
    expect(chat.hasMore).toBe(false);
    const read = readTurns([null, 7, { id: '', ord: 1 }, { id: 'x', ord: 0 }, { id: 'y', ord: 2, role: 'system', tools: 'no', questions: { id: '' } }]);
    expect(read).toHaveLength(1);
    expect(read[0]).toMatchObject({ id: 'y', role: 'assistant', tools: [], questions: null, attachments: [] });
  });
});

const card: WireQuestions = {
  id: 'qi-0123456789abcdef',
  state: 'pending',
  items: [
    { id: 'branch', header: 'Release', prompt: 'Which branch?', mode: 'single', required: true, allowsOther: true,
      options: [{ id: 'main', label: 'main', description: 'The default', recommended: true }, { id: 'next', label: 'next', description: null, recommended: false }] },
    { id: 'checks', header: 'Release', prompt: 'Which checks?', mode: 'multi', required: false, allowsOther: true,
      options: [{ id: 'tests', label: 'Tests', description: null, recommended: false }, { id: 'lint', label: 'Lint', description: null, recommended: false }] },
    { id: 'note', header: '', prompt: 'Anything else?', mode: 'text', required: false, allowsOther: false, options: [] },
  ],
};
// What the home sends for a confirmation, checkbox and verdicts card (each flattened to questions).
const confirmation: WireQuestions = { id: 'qi-1', state: 'pending', items: [
  { id: 'confirmation', header: 'Delete', prompt: 'Delete the draft?', mode: 'single', required: true, allowsOther: false,
    options: [{ id: 'accept', label: 'Delete', description: null, recommended: false }, { id: 'reject', label: 'Keep', description: null, recommended: false }] },
] };
const checkbox: WireQuestions = { id: 'qi-2', state: 'pending', items: [
  { id: 'checkbox', header: '', prompt: 'Which files?', mode: 'multi', required: true, allowsOther: false,
    options: [{ id: 'a', label: 'a.md', description: null, recommended: false }, { id: 'b', label: 'b.md', description: null, recommended: false }] },
] };
const verdicts: WireQuestions = { id: 'qi-3', state: 'pending', items: ['one', 'two'].map(id => ({
  id, header: 'Review', prompt: `Item ${id}`, mode: 'single' as const, required: true, allowsOther: false,
  options: ['approve', 'reject', 'defer'].map(v => ({ id: v, label: v, description: null, recommended: false })),
})) };

describe('the card adapter', () => {
  const asked = readTurns([turn(3, 'assistant', { questions: card })])[0];

  it('finds the open card and renders it as a questions card the QuestionCard takes', () => {
    const turns = readTurns([turn(2, 'assistant', { questions: { ...card, id: 'qi-old', state: 'closed' } }), turn(3, 'user'), turn(3, 'assistant', { questions: card })]);
    expect(openCardOf(turns)?.questions.id).toBe(card.id);
    expect(openCardOf(turns.slice(0, 2))).toBeNull();
    const c = wireQuestionsToCard(KEY, asked, asked.questions!, 'Mac mini');
    expect(c).toMatchObject({ id: card.id, sessionKey: KEY, turnOrd: 3, mode: 'questions', status: 'pending', source: 'mcp', nativeSessionId: null });
    expect(c.request.questions!.map(q => [q.id, q.answerMode, q.allowCustom, q.required])).toEqual([
      ['branch', 'single', true, true], ['checks', 'multi', true, false], ['note', 'text', false, false],
    ]);
    expect(c.request.questions![0].options[0]).toEqual({ id: 'main', label: 'main', description: 'The default', recommended: true });
    expect(c.request.questions![2].header).toBeUndefined();
    const closed = wireQuestionsToCard(KEY, asked, { ...card, state: 'closed' }, 'Mac mini');
    expect(closed.status).toBe('stale');
    expect(closed.statusReason).toBe('This card was answered or replaced on Mac mini.');
  });

  it('turns every answer mode back into the route\'s answers', () => {
    const c = wireQuestionsToCard(KEY, asked, card, 'Mac mini');
    const form = initialFormState(c.request);
    form.answers.branch = { ...form.answers.branch, selected: ['next'] };
    form.answers.checks = { ...form.answers.checks, selected: ['tests', 'lint'], otherOn: true, other: ' docs ' };
    form.answers.note = { ...form.answers.note, text: '  ship it  ' };
    expect(cardAnswerToWire(card, buildAnswer(c.request, form))).toEqual({ branch: 'next', checks: ['tests', 'lint', 'docs'], note: 'ship it' });
    // The person's own words for a single question; nothing picked is null.
    expect(cardAnswerToWire(card, { answers: { branch: { other: 'release-5' } } })).toEqual({ branch: 'release-5', checks: null, note: null });
    // Words where the card takes none are dropped.
    expect(cardAnswerToWire(confirmation, { answers: { confirmation: { other: 'maybe' } } })).toEqual({ confirmation: null });
    expect(cardAnswerToWire(confirmation, { answers: { confirmation: { selected: ['accept'] } } })).toEqual({ confirmation: 'accept' });
    expect(cardAnswerToWire(checkbox, { answers: { checkbox: { selected: ['a', 'b'] } } })).toEqual({ checkbox: ['a', 'b'] });
    expect(cardAnswerToWire(verdicts, { answers: { one: { selected: ['approve'] }, two: { selected: ['defer'] } } })).toEqual({ one: 'approve', two: 'defer' });
  });
});

describe('the open chat\'s stream (review fixes)', () => {
  const calls = () => vi.mocked(invoke).mock.calls.map(([c, a]) => `${c} ${(a as { key?: string })?.key ?? ''}`);
  const tick = () => new Promise(r => setTimeout(r, 0));

  it('never watches a chat closed while its turns load, and unwatches after any watch', async () => {
    vi.mocked(invoke).mockReset();
    let release!: (d: PeerChatDetail) => void;
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail'
      ? new Promise(r => { release = r as never; })
      : Promise.resolve(undefined)) as never);
    const opening = openRemoteChat(M, KEY);
    closeRemoteChat();
    release(detail([1]));
    await opening;
    await tick();
    expect(calls()).toEqual([`peer_detail ${KEY}`, `peer_unwatch ${KEY}`]);
    expect(remoteChat()).toBeNull();

    // Opened, then closed at once after its watch went out: watch, then unwatch, in order.
    vi.mocked(invoke).mockReset();
    let watched!: () => void;
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail' ? Promise.resolve(detail([1]))
      : command === 'peer_watch' ? new Promise<void>(r => { watched = r; }) : Promise.resolve(undefined)) as never);
    const second = openRemoteChat(M, KEY);
    await tick();
    closeRemoteChat();
    await tick();
    expect(calls()).toEqual([`peer_detail ${KEY}`, `peer_watch ${KEY}`]);
    watched();
    await second;
    await tick();
    expect(calls()).toEqual([`peer_detail ${KEY}`, `peer_watch ${KEY}`, `peer_unwatch ${KEY}`]);
  });

  it('a failure marks only the chat it was for offline', async () => {
    vi.mocked(invoke).mockReset();
    let fail!: (e: unknown) => void;
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_send'
      ? new Promise((_, reject) => { fail = reject; })
      : command === 'peer_detail' ? Promise.resolve(detail([1])) : Promise.resolve(undefined)) as never);
    await openRemoteChat(M, KEY);
    const sending = sendRemote('hello').catch(e => e);
    // Another chat opens before the send fails.
    await openRemoteChat(M, 's-other');
    fail({ code: 'offline', message: "Can't reach Mac mini.", state: 'offline', attemptsLeft: null });
    expect(await sending).toMatchObject({ code: 'offline' });
    expect(remoteChat()?.key).toBe('s-other');
    expect(remoteChat()?.offline).toBeNull();
    // The same failure for the open chat marks it.
    const again = sendRemote('hello').catch(e => e);
    fail({ code: 'offline', message: "Can't reach Mac mini.", state: 'offline', attemptsLeft: null });
    await again;
    expect(remoteChat()?.offline).toBe("Can't reach Mac mini.");
    closeRemoteChat();
  });
});

describe('round 2 review fixes', () => {
  const calls = () => vi.mocked(invoke).mock.calls.map(([c]) => c as string);
  const tick = () => new Promise(r => setTimeout(r, 0));
  const offline = { code: 'offline', message: 'The tunnel closed.', state: 'offline', attemptsLeft: null };

  it('a watch that fails shows the chat offline', async () => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail' ? Promise.resolve(detail([1]))
      : command === 'peer_watch' ? Promise.reject(offline) : Promise.resolve(undefined)) as never);
    await openRemoteChat(M, KEY);
    expect(remoteChat()?.offline).toBe('The tunnel closed.');
    closeRemoteChat();
  });

  it('a watch that hangs fails after the timeout, and the unwatch after it still goes', async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(invoke).mockReset();
      vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail' ? Promise.resolve(detail([1]))
        : command === 'peer_watch' ? new Promise(() => {}) : Promise.resolve(undefined)) as never);
      const opening = openRemoteChat(M, KEY);
      await vi.advanceTimersByTimeAsync(10);
      expect(calls()).toEqual(['peer_detail', 'peer_watch']);
      await vi.advanceTimersByTimeAsync(STREAM_CALL_TIMEOUT_MS);
      await opening;
      expect(remoteChat()?.offline).toMatch(/15 seconds/);
      closeRemoteChat();
      await vi.advanceTimersByTimeAsync(10);
      expect(calls()).toEqual(['peer_detail', 'peer_watch', 'peer_unwatch']);
    } finally {
      vi.useRealTimers();
    }
  });

  it('an answer fetches the chat again, so the card closes without its event', async () => {
    vi.mocked(invoke).mockReset();
    const asked = detail([1]);
    (asked.detail.turns[1] as Record<string, unknown>).questions = { id: 'q-1', state: 'pending', items: [{ id: 'a', header: '', prompt: 'Which?', mode: 'text', required: true, allowsOther: false, options: [] }] };
    let answered = false;
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail' ? Promise.resolve(answered ? detail([1]) : asked)
      : command === 'peer_answer' ? (answered = true, Promise.resolve({ accepted: true, state: 'queued', queueId: 1, waitingFor: null, forwardedTo: null }))
      : Promise.resolve(undefined)) as never);
    await openRemoteChat(M, KEY);
    const card = openCardOf(remoteChat()!.turns)!;
    expect(card.questions.id).toBe('q-1');
    await answerRemote(card.questions, { answers: { a: { selected: [], other: '', otherOn: false, text: 'This one' } } } as never);
    await tick();
    expect(calls().filter(c => c === 'peer_detail')).toHaveLength(2);
    expect(openCardOf(remoteChat()!.turns)).toBeNull();
    closeRemoteChat();
  });

  it('opening the same chat again quickly watches it once, for the newest open', async () => {
    vi.mocked(invoke).mockReset();
    const releases: ((d: PeerChatDetail) => void)[] = [];
    vi.mocked(invoke).mockImplementation(((command: string) => command === 'peer_detail'
      ? new Promise(r => { releases.push(r as never); })
      : Promise.resolve(undefined)) as never);
    const first = openRemoteChat(M, KEY);
    const second = openRemoteChat(M, KEY);
    releases[1](detail([2]));
    releases[0](detail([1]));
    await Promise.all([first, second]);
    await tick();
    expect(calls().filter(c => c === 'peer_watch')).toHaveLength(1);
    expect(remoteChat()?.turns.map(t => t.ord)).toEqual([2, 2]);
    closeRemoteChat();
  });
});

describe('PL1: a retried send keeps its id', () => {
  const offline = { code: 'offline', message: 'The tunnel closed.', state: 'offline', attemptsLeft: null };
  const ids = () => vi.mocked(invoke).mock.calls
    .filter(([c]) => c === 'peer_send')
    .map(([, a]) => (a as { clientMessageId: string }).clientMessageId);

  it('reuses the id after an offline failure and makes a new one after an answer', async () => {
    vi.mocked(invoke).mockReset();
    let sendOutcome: 'offline' | 'ok' | 'busy' = 'offline';
    vi.mocked(invoke).mockImplementation(((command: string) => {
      if (command === 'peer_detail') return Promise.resolve(detail([1]));
      if (command !== 'peer_send') return Promise.resolve(undefined);
      if (sendOutcome === 'offline') return Promise.reject(offline);
      if (sendOutcome === 'busy') return Promise.reject({ ...offline, code: 'busy', state: 'online' });
      return Promise.resolve({ accepted: true, state: 'queued', queueId: 1 });
    }) as never);
    await openRemoteChat(M, KEY);
    await sendRemote('ship it').catch(() => {});
    sendOutcome = 'ok';
    await sendRemote('ship it');
    // The retry of the same text is the same command.
    expect(ids()[1]).toBe(ids()[0]);
    // Answered: the next send of that text is a new command.
    await sendRemote('ship it');
    expect(ids()[2]).not.toBe(ids()[1]);
    // Other text is another command.
    sendOutcome = 'offline';
    await sendRemote('other').catch(() => {});
    expect(ids()[3]).not.toBe(ids()[2]);
    // A refusal forgets the id too.
    sendOutcome = 'busy';
    await sendRemote('other').catch(() => {});
    expect(ids()[4]).toBe(ids()[3]);
    await sendRemote('other').catch(() => {});
    expect(ids()[5]).not.toBe(ids()[4]);
    closeRemoteChat();
  });
});
