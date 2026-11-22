import { describe, expect, it } from 'vitest';
import { busyChoiceDetail, busyChoiceLabel, busyHint, busyMenuItems, defaultBusyChoice, steerBlockedWhy, steerRefusalNotice, steerRowsOf, steerTooLong, steerStateText, steerTurn } from './chatSteer';
import { turnsFromConversation } from './chatHydrate';
import type { AgentConversation } from './types';

describe('mid-turn steering in the composer', () => {
  it('Enter steers where the agent takes messages mid-turn, else queues', () => {
    expect(defaultBusyChoice(true, false)).toBe('steer');
    expect(defaultBusyChoice(false, false)).toBe('queue');
    // Images can't ride a steer.
    expect(defaultBusyChoice(true, true)).toBe('queue');
  });

  it('says what Enter will do, and why an agent queues', () => {
    expect(busyHint('steer', 'Claude', '')).toBe('Enter steers Claude at its next step · Send options has Queue for after and Stop and send');
    expect(busyHint('queue', 'Grok', steerBlockedWhy('Grok', false), '⌘Enter')).toBe("⌘Enter queues it for after this turn (Grok can't take a message mid-turn) · Send options has Queue for after and Stop and send");
    // P1-27: with a stop key, the hint also says how to stop (the chat
    // passes only a ⌘ chord: Esc doesn't stop while the box holds text).
    expect(busyHint('steer', 'Claude', '', 'Enter', '⌘.')).toBe('Enter steers Claude at its next step · ⌘. stops · Send options has Queue for after and Stop and send');
    expect(busyChoiceLabel('stop')).toBe('Stop and send');
    expect(busyChoiceDetail('stop', 'Codex')).toContain('Stops Codex, then sends it');
  });

  it('a steer is the person’s quiet row with its state in words', () => {
    const row = steerTurn({ id: 'st-1', text: 'Also the tests', at: '2026-10-21T19:02:00Z', afterTools: 2, state: 'sent' });
    expect(row).toMatchObject({ role: 'system', notice: 'steer', text: 'Also the tests', steer: { id: 'st-1', afterTools: 2, state: 'sent' } });
    expect(steerStateText('delivered')).toBe('Taken into this turn');
    expect(steerStateText('notDelivered')).toBe('Not taken: the turn ended first');
    expect(steerRowsOf({})).toEqual([]);
  });

  it('a steer the agent refused shows its reason (P1-30)', () => {
    const row = steerTurn({ id: 'st-2', text: 'Also lint', at: '2026-10-21T19:02:00Z', afterTools: 0, state: 'notDelivered', reason: 'no active turn to steer' });
    expect(row.steer).toMatchObject({ state: 'notDelivered', reason: 'no active turn to steer' });
    expect(steerStateText('notDelivered', 'no active turn to steer')).toBe('Not taken: the agent said “no active turn to steer”');
  });
});

describe('steering tells the truth (P1-30)', () => {
  const codes = ['STEER_NOT_RUNNING', 'STEER_ENDING', 'STEER_UNSUPPORTED', 'STEER_INVALID', 'STEER_NOT_STORED', 'STEER_SOMETHING_NEW', 'Error: STEER_INVALID'];

  it('no refusal shows a STEER_ code', () => {
    for (const code of codes) expect(steerRefusalNotice(code, 'Codex').text).not.toContain('STEER_');
  });

  it('each refusal says what happened and whether the message goes next', () => {
    expect(steerRefusalNotice('STEER_NOT_RUNNING', 'Claude')).toEqual({ fallsBack: true, text: expect.stringContaining("hasn't started") });
    expect(steerRefusalNotice('STEER_ENDING', 'Claude')).toEqual({ fallsBack: true, text: expect.stringContaining('was finishing') });
    expect(steerRefusalNotice('STEER_UNSUPPORTED', 'Codex')).toEqual({ fallsBack: true, text: expect.stringContaining("Codex can't take a message mid-turn") });
    const invalid = steerRefusalNotice('STEER_INVALID', 'Claude');
    expect(invalid.fallsBack).toBe(false);
    expect(invalid.text).toContain('3,000 characters');
    expect(invalid.text).toContain('20');
    expect(steerRefusalNotice('STEER_NOT_STORED', 'Claude')).toEqual({ fallsBack: false, text: expect.stringContaining("couldn't save") });
    // Not a steer code at all (a transport error): its own words, once.
    expect(steerRefusalNotice('Error: the window closed', 'Claude')).toEqual({ fallsBack: false, text: expect.stringContaining('the window closed') });
  });

  it('a draft over 3,000 characters queues by default, and the hint says why', () => {
    expect(defaultBusyChoice(true, false, { chars: 3_000 })).toBe('steer');
    expect(defaultBusyChoice(true, false, { chars: 3_001 })).toBe('queue');
    const why = steerBlockedWhy('Claude', true, { chars: 3_001 });
    expect(busyHint(defaultBusyChoice(true, false, { chars: 3_001 }), 'Claude', why)).toMatch(/^Enter queues it for after this turn \(.*3,000 characters/);
  });

  it('review: "too long" mirrors the backend, characters and JSON bytes both', () => {
    expect(steerTooLong('a'.repeat(3_000))).toBe(false);
    expect(steerTooLong(`  ${'a'.repeat(3_000)}  `)).toBe(false);
    expect(steerTooLong('a'.repeat(3_001))).toBe(true);
    // 2,400 CJK characters are 7,200 bytes as JSON; 1,500 escapes are 9,000.
    expect(steerTooLong('漢'.repeat(2_400))).toBe(true);
    expect(steerTooLong('\u001b'.repeat(1_500))).toBe(true);
    expect(steerBlockedWhy('Claude', true, { tooLong: true })).toContain('too long to steer');
    expect(steerRefusalNotice('STEER_INVALID', 'Claude').text).toContain('too long');
  });

  it('Send options says it is still checking an agent’s steer support', () => {
    expect(busyMenuItems('Codex', false, { empty: false, images: false, note: steerBlockedWhy('Codex', false, { unknown: true }) }, () => {})[0].label).toContain('checking whether Codex');
  });

  it('a turn still queued, an image and unknown support each queue with a reason', () => {
    expect(defaultBusyChoice(true, false, { queued: true })).toBe('queue');
    expect(steerBlockedWhy('Claude', true, { queued: true })).toContain("hasn't started");
    expect(steerBlockedWhy('Claude', true, { images: true })).toContain('image');
    // C5: an attached file rules Steer now out too, in its own words.
    expect(steerBlockedWhy('Claude', true, { files: true })).toBe("an attached file can't go into a running turn");
    expect(defaultBusyChoice(true, false, { files: true })).toBe('queue');
    // Repo ideas 2026-10-27 #3: a shared page's text waits for the turn too.
    expect(steerBlockedWhy('Claude', true, { pages: true })).toBe("shared page text can't go into a running turn");
    expect(defaultBusyChoice(true, false, { pages: true })).toBe('queue');
    expect(defaultBusyChoice(false, false, { unknown: true })).toBe('queue');
    expect(steerBlockedWhy('Codex', false, { unknown: true })).toContain('checking whether Codex');
    expect(steerBlockedWhy('Claude', true)).toBe('');
  });

  it('Send options says why Steer now is off, in the backend’s words for an agent without one', () => {
    const note = "Grok can't take a message mid-turn. Queue it for after, or Stop and send restarts it with your message.";
    const none = busyMenuItems('Grok', false, { empty: false, images: false, note }, () => {});
    expect(none[0]).toMatchObject({ label: note, informational: true });
    const images = busyMenuItems('Claude', true, { empty: false, images: true }, () => {});
    expect(images[0]).toMatchObject({ disabled: true, label: expect.stringContaining('image') });
  });

  it('a restored turn shows its steers before its answer', () => {
    const conversation = {
      turns: [
        { role: 'user', text: 'Refactor it', tools: [], agent: 'claude', status: 'success' },
        { role: 'assistant', text: 'Done.', tools: [], agent: 'claude', status: 'success',
          steers: [{ id: 'st-9', text: 'Keep the API', at: '2026-10-21T19:00:00Z', afterTools: 1, state: 'delivered' }] },
      ],
    } as unknown as AgentConversation;
    const turns = turnsFromConversation(conversation);
    expect(turns.map(t => [t.role, t.notice ?? null, t.text])).toEqual([
      ['user', null, 'Refactor it'],
      ['system', 'steer', 'Keep the API'],
      ['assistant', null, 'Done.'],
    ]);
  });
});
