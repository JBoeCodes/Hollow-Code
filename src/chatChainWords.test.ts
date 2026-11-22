import { describe, expect, it } from 'vitest';
import { earlierLabel, forwardWords, oneMainConversation } from './chatChainWords';
import { chainCardKeys, continuedIn } from './store/chatChainState';
import { applyInteractionUpdate } from './questionCardList';
import type { ChatInteraction } from './types';

// One conversation from the person's side (2026-11-07): the chain helpers.

const view = (key: string, current: string, earlier: string[] = []) => ({
  key, current, currentTitle: current === 'b' ? 'Main, after the handover' : null, main: true,
  earlier: earlier.map((k, i) => ({ key: k, title: null, createdAt: `2026-11-0${i + 1}T10:00:00Z` })),
  forwards: [],
});

describe('conversation chains', () => {
  it('says where a conversation continued, and nothing while it is current', () => {
    expect(continuedIn(view('a', 'b'))).toEqual({ key: 'b', title: 'Main, after the handover' });
    expect(continuedIn(view('c', 'd'))).toEqual({ key: 'd', title: 'its current chat' });
    expect(continuedIn(view('b', 'b'))).toBeNull();
    expect(continuedIn(null)).toBeNull();
  });

  it("shows the earlier chats' cards only in the current one", () => {
    expect([...chainCardKeys('b', view('b', 'b', ['a', 'z']))].sort()).toEqual(['a', 'b', 'z']);
    // An earlier chat shows its own cards, not the chain's.
    expect([...chainCardKeys('a', view('a', 'b', ['z']))]).toEqual(['a']);
    // A view for another conversation (still loading) adds nothing.
    expect([...chainCardKeys('b', view('x', 'x', ['a']))]).toEqual(['b']);
    expect(chainCardKeys(null, view('b', 'b', ['a'])).size).toBe(0);
  });

  it('applies a card from an earlier chat of the chain to the current one', () => {
    const card = { id: 'q1', sessionKey: 'a', status: 'pending' } as ChatInteraction;
    expect(applyInteractionUpdate([], card, 'b')).toEqual([]);
    expect(applyInteractionUpdate([], card, 'b', new Set(['a', 'b']))).toEqual([card]);
    const answered = { ...card, status: 'answered' } as ChatInteraction;
    expect(applyInteractionUpdate([card], answered, 'b', new Set(['a', 'b']))).toEqual([]);
  });

  it('lists the COO once, by its newest conversation', () => {
    const rows = [
      { key: 'm1', createdAt: '2026-11-07T09:00:00Z', cwd: 'main' },
      { key: 'w1', createdAt: '2026-11-07T10:00:00Z', cwd: 'work' },
      { key: 'm2', createdAt: '2026-11-07T14:04:00Z', cwd: 'main' },
      { key: 'm0', createdAt: '2026-11-06T09:00:00Z', cwd: 'main' },
    ];
    expect(oneMainConversation(rows, r => r.cwd === 'main').map(r => r.key)).toEqual(['w1', 'm2']);
    expect(oneMainConversation(rows, () => false)).toHaveLength(4);
  });

  it('words the forward notes from each side, with no dashes', () => {
    const note = { from: 'old', to: 'new', at: '2026-11-07T21:18:00Z', preview: 'Install' };
    const at = () => '4:18 PM';
    const there = forwardWords(note, 'old', at);
    const here = forwardWords(note, 'new', at);
    expect(there).toBe('“Install” was sent here after this conversation continued, so it went there (4:18 PM).');
    expect(here).toBe('“Install” was sent in an earlier chat after it continued here, so it came here (4:18 PM).');
    expect(earlierLabel(1)).toBe('Earlier in this conversation (1 chat)');
    expect(earlierLabel(3)).toBe('Earlier in this conversation (3 chats)');
    for (const text of [there, here, earlierLabel(2)]) expect(text).not.toMatch(/[\u2013\u2014]/);
  });
});
