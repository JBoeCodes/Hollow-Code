import { describe, expect, it } from 'vitest';
import { CARD_ANSWER_HEADER, carriesAttachments, editIndex, hostWrittenTurn, caretEdges, recallAnchor, recallList, repeatable, sentTime, type EditTurn } from './messageActions';
import { attachedFilesBlock } from './composerFiles';
import AGENT_OUT_RS from '../src-tauri/src/agent_out.rs?raw';
import INTERACTION_SCHEMA_RS from '../src-tauri/src/interaction_schema.rs?raw';
import { CONTINUE_PROMPT, regenerateIndex, retryIndex, type RetryTurn } from './chatContinue';
import { WAKE_PREFIX } from './mainAgent';
import { canvasAnswerText } from './canvas/askAnswer';
import { markedReplyAt, savedTurnAt, savedUserAt, turnsFromConversation } from './chatHydrate';
import type { AgentConversation } from './types';

describe('Edit your last message (overnight 2026-11-04 C3)', () => {
  const free = { busy: false, queued: false };
  const ask: EditTurn = { role: 'user', text: 'Draft the release notes.' };
  const reply: EditTurn = { role: 'assistant', text: 'Here are the notes.' };

  it('offers your last message only', () => {
    expect(editIndex([ask, reply], free)).toBe(0);
    expect(editIndex([ask, reply, { role: 'user', text: 'Shorter, please.' }, reply, { role: 'system', text: 'Stopped.' }], free)).toBe(2);
    // A message still waiting for its answer is the last one too.
    expect(editIndex([ask], free)).toBe(0);
  });

  it('is not offered while a turn runs or a message is queued', () => {
    expect(editIndex([ask, reply], { ...free, busy: true })).toBe(-1);
    expect(editIndex([ask, reply], { ...free, queued: true })).toBe(-1);
  });

  it('never offers a message that is not yours to edit, or one with no words', () => {
    const wake: EditTurn = { role: 'user', text: 'wake' };
    // The last message decides: an earlier one is never offered instead.
    expect(editIndex([ask, reply, wake, reply], free, t => t === wake)).toBe(-1);
    expect(editIndex([ask, reply, { role: 'user', text: '   ' }], free)).toBe(-1);
    expect(editIndex([reply], free)).toBe(-1);
    expect(editIndex([], free)).toBe(-1);
  });
});

describe('which messages Retry and Edit can send again (C3 review)', () => {
  type Turn = RetryTurn & { text: string; images?: string[]; files?: { name: string; path: string }[]; sharedPage?: boolean };
  const free = { busy: false, queued: false };
  const user = (text: string, extra: { images?: string[]; files?: { name: string; path: string }[]; sharedPage?: boolean } = {}): Turn => ({ role: 'user', text, ...extra });
  const wake = user(`${WAKE_PREFIX} check in`);
  const cont = user(CONTINUE_PROMPT);
  const card = user('Answered questions\n- Install main: Not now');
  const canvas = user(canvasAnswerText({ canvasId: 'release-choices', revision: 1, answers: {} }, 'Ship day: Friday'));
  const image = user('What is wrong here?', { images: ['data:image/png;base64,AAAA'] });
  const page = user('Summarize this page', { sharedPage: true });
  const file = user('Check this sheet', { files: [{ name: 'q3.xlsx', path: '/tmp/.hollow-attachments/q3.xlsx' }] });
  const plain = user('Draft the release notes.');
  // The continuation on another agent, as `agent_out::continue_prompt`
  // words it: its message, and the clause that says when the agent has room.
  const continueFn = AGENT_OUT_RS.slice(AGENT_OUT_RS.indexOf('fn continue_prompt'));
  const continuePrompt = /"(\{from\} ran out of usage(?:[^"\\]|\\.)*)"/.exec(continueFn)?.[1] ?? '';
  const roomAgain = /\.map\(\|at\| format!\("((?:[^"\\]|\\.)*)"\)\)/.exec(continueFn)?.[1] ?? '';
  const handoff = user(continuePrompt.replace('{from}', 'Claude').replace('{when}', '').replace('{to}', 'Codex'));

  it('reads the card header and the continuation as the backend writes them', () => {
    expect(/pub const ANSWER_HEADER: &str = "([^"]*)";/.exec(INTERACTION_SCHEMA_RS)?.[1]).toBe(CARD_ANSWER_HEADER);
    expect(continuePrompt).toContain('{to}');
    expect(roomAgain).toContain('{at}');
    for (const when of ['', roomAgain.replace('{at}', '3:00 PM')]) {
      const text = continuePrompt.replace('{from}', 'Claude').replace('{when}', when).replace('{to}', 'Codex');
      expect(text, 'every placeholder filled').not.toMatch(/[{}]/);
      expect(hostWrittenTurn(user(text)), when).toBe(true);
    }
    expect(hostWrittenTurn(user('Claude ran out of usage again. Can Codex continue that request here?'))).toBe(false);
  });

  it('knows the messages Hollow wrote', () => {
    for (const t of [wake, cont, card, canvas, handoff]) expect(hostWrittenTurn(t), t.text).toBe(true);
    expect(hostWrittenTurn(user('Answered questions'))).toBe(true);
    for (const t of [plain, image, page, file, user('Answered questions about the launch, please')]) expect(hostWrittenTurn(t), t.text).toBe(false);
  });

  it('knows a message that went with more than its words', () => {
    expect(carriesAttachments(image)).toBe(true);
    expect(carriesAttachments(page)).toBe(true);
    expect(carriesAttachments(file)).toBe(true);
    expect(carriesAttachments(user(`Read it\n\n${attachedFilesBlock([{ name: 'q3.xlsx', path: '/tmp/q3.xlsx' }])}`))).toBe(true);
    expect(carriesAttachments(user('Look\n\n<attached-images count="1">\n1. /tmp/a.png\n</attached-images>'))).toBe(true);
    expect(carriesAttachments(user('<shared-page url="https://example.com">\nText\n</shared-page>\n\nSummarize'))).toBe(true);
    expect(carriesAttachments(plain)).toBe(false);
    expect(carriesAttachments(user('What does <shared-pages> mean?', { images: [] }))).toBe(false);
  });

  it('offers neither Edit nor either Retry for one, and both for your own words', () => {
    const skip = (t: Turn) => !repeatable(t);
    const done: Turn = { role: 'assistant', text: 'Done.', status: 'success' };
    const failed: Turn = { role: 'assistant', text: '(error: spawn failed)', status: 'error' };
    for (const t of [wake, cont, card, canvas, handoff, image, page, file]) {
      expect(editIndex([t, done], free, skip), t.text).toBe(-1);
      expect(regenerateIndex([t, done], free, skip), t.text).toBeNull();
      expect(retryIndex([t, failed], free, skip), t.text).toBe(-1);
    }
    expect(editIndex([plain, done], free, skip)).toBe(0);
    expect(regenerateIndex([plain, done], free, skip)).toEqual({ reply: 1, ask: 0 });
    expect(retryIndex([plain, failed], free, skip)).toBe(0);
  });
});

describe('when your message was sent', () => {
  const at = new Date(2026, 10, 3, 22, 21).toISOString();

  it('reads like the reply footer: the time today, the date on an older day', () => {
    expect(sentTime(at, new Date(2026, 10, 3, 23, 0).getTime(), 'en-US')?.label).toBe('10:21 PM');
    expect(sentTime(at, new Date(2026, 10, 4, 9, 0).getTime(), 'en-US')?.label).toBe('Nov 3, 10:21 PM');
  });

  it('gives the full date and time for its tooltip', () => {
    const title = sentTime(at, Date.now(), 'en-US')?.title ?? '';
    expect(title).toMatch(/^Sent Tuesday, November 3, 2026/);
    expect(title).toContain('10:21');
  });

  it('shows nothing rather than a wrong time', () => {
    expect(sentTime(undefined)).toBeNull();
    expect(sentTime('not a time')).toBeNull();
  });

  it('takes a saved user turn’s start as its send time, and nothing for a reply', () => {
    expect(savedUserAt({ role: 'user', startedAt: '2026-11-03T22:21:00Z' })).toBe('2026-11-03T22:21:00Z');
    expect(savedUserAt({ role: 'assistant', startedAt: '2026-11-03T22:21:00Z' })).toBeUndefined();
    expect(savedUserAt({ role: 'user', startedAt: '' })).toBeUndefined();
    expect(savedUserAt({ role: 'user', startedAt: 'garbage' })).toBeUndefined();
    expect(savedUserAt({ role: 'user' })).toBeUndefined();
  });

  it('a loaded conversation gives your messages their time and replies none', () => {
    const conversation = {
      turns: [
        { role: 'user', text: 'Refactor it', tools: [], agent: 'claude', status: 'success', startedAt: '2026-11-03T22:21:00Z' },
        { role: 'assistant', text: 'Done.', tools: [], agent: 'claude', status: 'success', startedAt: '2026-11-03T22:21:00Z' },
      ],
    } as unknown as AgentConversation;
    expect(turnsFromConversation(conversation).map(t => t.at)).toEqual(['2026-11-03T22:21:00Z', undefined]);
  });

  it('a saved reply takes its turn’s end, so its row says when it finished (C8)', () => {
    expect(savedTurnAt({ role: 'assistant', startedAt: '2026-11-03T22:21:00Z', endedAt: '2026-11-03T22:23:10Z' })).toBe('2026-11-03T22:23:10Z');
    expect(savedTurnAt({ role: 'assistant', startedAt: '2026-11-03T22:21:00Z', endedAt: null })).toBeUndefined();
    expect(savedTurnAt({ role: 'assistant', startedAt: '2026-11-03T22:21:00Z', endedAt: 'garbage' })).toBeUndefined();
    expect(savedTurnAt({ role: 'user', startedAt: '2026-11-03T22:21:00Z', endedAt: '2026-11-03T22:23:10Z' })).toBe('2026-11-03T22:21:00Z');
    for (const status of ['success', 'error', 'cancelled']) expect(savedTurnAt({ role: 'assistant', status, endedAt: '2026-11-03T22:23:10Z' })).toBe('2026-11-03T22:23:10Z');
    // An interrupted turn's end is the recovery time at the next launch.
    expect(savedTurnAt({ role: 'assistant', status: 'interrupted', endedAt: '2026-11-04T08:00:00Z' })).toBeUndefined();
    expect(savedTurnAt({ role: 'assistant', status: 'running', endedAt: '2026-11-04T08:00:00Z' })).toBeUndefined();
  });

  it('a live reply keeps its first finish time, except that a success replaces an error’s', () => {
    expect(markedReplyAt({}, 'error', 'T2')).toBe('T2');
    expect(markedReplyAt({ status: 'error', at: 'T1' }, 'error', 'T2')).toBe('T1');
    expect(markedReplyAt({ status: 'success', at: 'T1' }, 'error', 'T2')).toBe('T1');
    expect(markedReplyAt({ status: 'error', at: 'T1' }, 'success', 'T2')).toBe('T2');
    expect(markedReplyAt({ status: 'success', at: 'T1' }, 'success', 'T2')).toBe('T1');
    const conversation = {
      turns: [
        { role: 'user', text: 'Refactor it', tools: [], agent: 'claude', status: 'success', startedAt: '2026-11-03T22:21:00Z', endedAt: '2026-11-03T22:23:10Z' },
        { role: 'assistant', text: 'Done.', tools: [], agent: 'claude', status: 'success', startedAt: '2026-11-03T22:21:00Z', endedAt: '2026-11-03T22:23:10Z' },
      ],
    } as unknown as AgentConversation;
    expect(turnsFromConversation(conversation).map(t => t.at)).toEqual(['2026-11-03T22:21:00Z', '2026-11-03T22:23:10Z']);
  });
});

describe('Up-arrow recall (overnight 2026-11-07, D1)', () => {
  const ask = (text: string) => ({ role: 'user', text });
  const reply = { role: 'assistant', text: 'Done.' };

  it('lists this conversation\'s messages newest first, then the earlier prompts from every chat', () => {
    expect(recallList([ask('first'), reply, ask('second'), reply], ['elsewhere one', 'elsewhere two'])).toEqual(['second', 'first', 'elsewhere one', 'elsewhere two']);
  });

  it('shows a repeated message once, at its newest place', () => {
    expect(recallList([ask('same'), reply, ask('other'), reply, ask('same')], ['other', 'older'])).toEqual(['same', 'other', 'older']);
  });

  it('is the earlier prompts alone for a conversation with no messages yet', () => {
    expect(recallList([], ['one', 'two'])).toEqual(['one', 'two']);
    expect(recallList([], [])).toEqual([]);
  });

  it('skips what Hollow wrote, system notices, empty messages and a message that holds an attachment block', () => {
    const turns = [
      ask('mine'),
      { role: 'system', text: 'Switched account' },
      ask(CONTINUE_PROMPT),
      ask(`${CARD_ANSWER_HEADER}\nWhich one?: This`),
      ask('   '),
      ask('Look at these\n<attached-files>\n- notes.txt\n</attached-files>'),
      reply,
    ];
    expect(recallList(turns, [])).toEqual(['mine']);
  });

  it('stops at the cap', () => {
    const turns = Array.from({ length: 5 }, (_, i) => ask(`message ${i}`));
    expect(recallList(turns, ['x', 'y'], 3)).toEqual(['message 4', 'message 3', 'message 2']);
  });
});

describe('Where an Up-arrow recall stands (overnight 2026-11-07, D1, Codex read)', () => {
  const list = ['C', 'B', 'A', 'earlier'];
  it('keeps the place while the box still shows the message there', () => {
    expect(recallAnchor(list, 2, 'A')).toBe(2);
    expect(recallAnchor(list, 0, 'C')).toBe(0);
  });
  it('follows the message when the list shifted under the place (a queued turn started)', () => {
    // The place was 1 when the list was [B, A, C]; C then moved to the front.
    expect(recallAnchor(list, 1, 'A')).toBe(2);
    // A place past the end of a shorter list finds the message where it is.
    expect(recallAnchor(list, 9, 'earlier')).toBe(3);
  });
  it('is not recalling when the box shows something else, or nothing', () => {
    expect(recallAnchor(list, 0, 'C\n\n> a quote')).toBeNull();
    expect(recallAnchor(list, 1, 'typed afterwards')).toBeNull();
    expect(recallAnchor(list, 1, '')).toBeNull();
  });
  it('is not recalling when there was no place, or the list is gone', () => {
    expect(recallAnchor(list, null, 'C')).toBeNull();
    expect(recallAnchor([], 0, 'C')).toBeNull();
  });
  it('reads a multi-line message the way the box does (CRLF to LF, trimmed)', () => {
    expect(recallAnchor(['one\r\ntwo'], 0, 'one\ntwo')).toBe(0);
    expect(recallAnchor(['one\ntwo'], 0, 'one\ntwo\n')).toBe(0);
  });
});

describe('caretEdges (ENG-93)', () => {
  // A stand-in layout: 10 characters a line, 20 px each.
  const height = (text: string) => text.split('\n').reduce((n, line) => n + Math.max(1, Math.ceil(line.length / 10)), 0) * 20;
  it('counts a wrapped line as more than one', () => {
    const long = 'a'.repeat(25);
    expect(caretEdges(long, 5, height)).toEqual({ atTop: true, atBottom: false });
    expect(caretEdges(long, 15, height)).toEqual({ atTop: false, atBottom: false });
    expect(caretEdges(long, 25, height)).toEqual({ atTop: false, atBottom: true });
  });
  it('falls back to newlines with no layout', () => {
    expect(caretEdges('one\ntwo', 2, () => 0)).toEqual({ atTop: true, atBottom: false });
    expect(caretEdges('one\ntwo', 6, () => 0)).toEqual({ atTop: false, atBottom: true });
    expect(caretEdges('', 0, () => 0)).toEqual({ atTop: true, atBottom: true });
  });
});
