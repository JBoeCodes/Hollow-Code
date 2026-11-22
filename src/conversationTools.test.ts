import chatBodySource from './components/ChatBody.tsx?raw';
import { describe, expect, it } from 'vitest';
import { TURN_WINDOW, branchIntoNewChat, chatTitleShown, forThisChat, conversationMarkdown, conversationNoteTitle, conversationOutline, findInTurns, handoffText, lastUserIndex, renamedTitle, scrollSignature, ScrollMemory, windowStart, type TurnLike } from './conversationTools';

const turns = [
  { role: 'user' as const, text: 'Draft the Q3 review from the exports.' },
  { role: 'assistant' as const, text: 'The draft is in outputs/q3-review.md.' },
  { role: 'system' as const, text: 'Switched account' },
  { role: 'user' as const, text: '\nRevise section 3 (labour).\nKeep it short.' },
  { role: 'assistant' as const, text: 'Section 3 now explains overtime.' },
];

describe('conversation navigation (next-level P04)', () => {
  it('finds matches in user and agent text, not system notices', () => {
    expect(findInTurns(turns, 'section 3')).toEqual([3, 4]);
    expect(findInTurns(turns, 'account')).toEqual([]);
    expect(findInTurns(turns, 'q')).toEqual([]);
  });
  it('outlines the user\'s prompts', () => {
    expect(conversationOutline(turns).map(e => [e.index, e.text])).toEqual([
      [0, 'Draft the Q3 review from the exports.'], [3, 'Revise section 3 (labour).'],
    ]);
  });
  it('renders the latest window and widens to reach an earlier turn', () => {
    expect(windowStart(1000, null)).toBe(1000 - TURN_WINDOW);
    expect(windowStart(1000, 920, 100)).toBe(96);
    expect(windowStart(10, null)).toBe(0);
  });
  it('labels a handoff as written context, not a resumed session', () => {
    const text = handoffText(turns, 4, { title: 'Q3 review', agentLabel: 'Claude' });
    expect(text).toMatch(/^Context handoff from “Q3 review” \(a written summary, not a resumed session of Claude\)/);
    expect(text).toContain('It started with: Draft the Q3 review');
    expect(text).toContain('- Agent: Section 3 now explains overtime.');
    expect(text).not.toContain('Switched account');
  });
  it('restores scroll only for the same conversation and turns', () => {
    expect(scrollSignature('k', turns)).toBe(scrollSignature('k', [...turns]));
    expect(scrollSignature('k', turns)).not.toBe(scrollSignature('k', turns.slice(0, 4)));
    expect(scrollSignature(null, turns)).toBe('');
  });
});

// Overnight C5: long conversations. A footer is the latest when no user turn
// follows it; one pass replaces a slice of the rest of the thread per row.
describe('lastUserIndex (overnight C5)', () => {
  const naive = (turns: { role: TurnLike['role'] }[], i: number) => !turns.slice(i + 1).some(t => t.role === 'user');
  it('marks the same footers as the per-row scan did', () => {
    const roles: TurnLike['role'][] = ['user', 'assistant', 'system'];
    let seed = 7;
    const next = () => (seed = (seed * 1103515245 + 12345) % 2147483648);
    for (let n = 0; n < 60; n++) {
      const turns = Array.from({ length: n }, () => ({ role: roles[next() % 3] }));
      const last = lastUserIndex(turns);
      for (let i = 0; i < n; i++) expect(i >= last, `n=${n} i=${i}`).toBe(naive(turns, i));
    }
  });

  it('is -1 with no user turn, and handles 2,000 turns in one pass', () => {
    expect(lastUserIndex([])).toBe(-1);
    expect(lastUserIndex([{ role: 'assistant' }, { role: 'system' }])).toBe(-1);
    const long = Array.from({ length: 2000 }, (_, i) => ({ role: (i % 2 ? 'assistant' : 'user') as TurnLike['role'] }));
    expect(lastUserIndex(long)).toBe(1998);
  });

  it('is what ChatBody uses for the footer, not a per-row slice', () => {
    expect(chatBodySource).toContain('latest={turnIdx() >= lastUser()}');
    expect(chatBodySource).not.toMatch(/chatTurns\.slice\(turnIdx\(\) \+ 1\)/);
  });
});

// P1-27: Copy conversation (Markdown) from Conversation tools.
describe('conversationMarkdown', () => {
  it('writes each speaker once per reply, the person’s steers included, without notices or hidden blocks', () => {
    const md = conversationMarkdown([
      { role: 'user', text: 'Draft the Q3 review.' },
      { role: 'assistant', text: 'Reading the exports.', agent: 'claude' },
      { role: 'system', text: 'Switched account' },
      { role: 'system', text: 'Use the July numbers', notice: 'steer', steer: { state: 'delivered' } },
      { role: 'system', text: 'Too late', notice: 'steer', steer: { state: 'notDelivered' } },
      { role: 'assistant', text: 'The draft is ready.\n\n```hollow-next\nMerge it\n```', agent: 'claude' },
      { role: 'user', text: 'Answered canvas "pick":\n- Layout: Grid\n\n```hollow-canvas-answer\n{"canvasId":"pick"}\n```' },
      { role: 'assistant', text: '', agent: 'codex' },
    ], { title: 'Q3 review', agentLabel: a => (a === 'claude' ? 'Claude' : 'Agent') });
    expect(md).toBe([
      '# Q3 review', '',
      '## You', '', 'Draft the Q3 review.', '',
      '## Claude', '', 'Reading the exports.', '',
      '## You', '', 'Use the July numbers', '',
      '## Claude', '', 'The draft is ready.', '',
      '## You', '', 'Answered canvas "pick":\n- Layout: Grid', '',
    ].join('\n'));
  });

  it('keeps a message’s first-line indentation, so a pasted indented code block stays code (Codex read)', () => {
    const md = conversationMarkdown([
      { role: 'user', text: '\n    def f():\n        return 1\nWhy None?\n' },
      { role: 'assistant', text: '   \n    return 1 is fine.\n', agent: 'claude' },
    ], { agentLabel: () => 'Claude' });
    expect(md).toBe(['## You', '', '    def f():\n        return 1\nWhy None?', '', '## Claude', '', '    return 1 is fine.', ''].join('\n'));
  });
});

// Overnight 10-11/12 P6: the scroll memory kept one entry per conversation
// and turn count for the app's lifetime.
describe('ScrollMemory', () => {
  it('keeps only the latest signature per conversation', () => {
    const m = new ScrollMemory<number>();
    m.set('a', 'a:1:x', 10);
    m.set('a', 'a:2:x', 20);
    m.set('a', 'a:3:x', 30);
    expect(m.size).toBe(1);
    expect(m.get('a:1:x')).toBeUndefined();
    expect(m.get('a:3:x')).toBe(30);
    m.set('a', 'a:3:x', 31);
    expect(m.get('a:3:x')).toBe(31);
  });
  it('holds at most `cap` conversations, oldest written dropped first', () => {
    const m = new ScrollMemory<number>(3);
    for (const k of ['a', 'b', 'c']) m.set(k, `${k}:1:x`, 1);
    m.set('a', 'a:2:x', 2);
    m.set('d', 'd:1:x', 1);
    expect(m.size).toBe(3);
    expect(m.get('b:1:x')).toBeUndefined();
    expect(m.get('a:2:x')).toBe(2);
    expect(m.get('d:1:x')).toBe(1);
  });
});

describe('a conversation saved as a note (overnight 2026-11-07, D1)', () => {
  it('is titled by the conversation\'s name, else the first thing asked, else "Conversation"', () => {
    expect(conversationNoteTitle('  Q3 review  ', turns)).toBe('Q3 review');
    expect(conversationNoteTitle(null, turns)).toBe('Draft the Q3 review from the exports.');
    expect(conversationNoteTitle('', [{ role: 'user', text: '\n# A heading first\nthen more' }])).toBe('A heading first');
    expect(conversationNoteTitle(undefined, [{ role: 'assistant', text: 'Hello' }])).toBe('Conversation');
    expect(conversationNoteTitle(undefined, [])).toBe('Conversation');
  });

  it('is one line of at most 120 characters', () => {
    expect(conversationNoteTitle('Two\n  lines   here', turns)).toBe('Two lines here');
    const long = conversationNoteTitle('x'.repeat(200), turns);
    expect(Array.from(long)).toHaveLength(120);
    expect(long.endsWith('…')).toBe(true);
  });
});

describe('the title above a chat (overnight 2026-11-07, D1)', () => {
  it('shows the saved title, the sidebar\'s word for an unnamed chat, and nothing for a chat the list does not hold', () => {
    expect(chatTitleShown({ title: '  Q3 review  ' })).toBe('Q3 review');
    expect(chatTitleShown({ title: null })).toBe('Conversation');
    expect(chatTitleShown({ title: '   ' })).toBe('Conversation');
    expect(chatTitleShown(null)).toBeNull();
    expect(chatTitleShown(undefined)).toBeNull();
  });

  it('renames to the trimmed name, and does nothing for an empty or unchanged one', () => {
    expect(renamedTitle('  Pricing, round two ', 'Review pricing')).toBe('Pricing, round two');
    expect(renamedTitle('Review pricing', 'Review pricing')).toBeNull();
    expect(renamedTitle('  Review pricing  ', ' Review pricing')).toBeNull();
    expect(renamedTitle('   ', 'Review pricing')).toBeNull();
    expect(renamedTitle('First name', null)).toBe('First name');
    expect(renamedTitle('x'.repeat(200), 'old')).toHaveLength(120);
    // ENG-93: 120 code points, not UTF-16 units, and never half an emoji.
    expect(Array.from(renamedTitle('😀'.repeat(130), 'old') ?? '')).toHaveLength(120);
    expect(Array.from(renamedTitle('😀'.repeat(70), 'old') ?? '')).toHaveLength(70);
    expect(renamedTitle(`${'x'.repeat(119)}e\u0301`, 'old')).toBe('x'.repeat(119));
  });
});

describe('the reply More menu stays with its own conversation (ENG-78)', () => {
  it('an action taken after a chat switch does nothing', () => {
    let key = 'conv-a';
    const at = key;
    const ran: number[] = [];
    const save = forThisChat(() => key === at, 3, i => ran.push(i));
    save();
    key = 'conv-b';
    save();
    expect(ran).toEqual([3]);
  });
  it('Continue from here leaves the draft alone when the new chat is refused', async () => {
    let draft = 'mine';
    await expect(branchIntoNewChat(async () => false, () => { draft = 'hand-off'; })).resolves.toBe(false);
    expect(draft).toBe('mine');
    await expect(branchIntoNewChat(async () => true, () => { draft = 'hand-off'; })).resolves.toBe(true);
    expect(draft).toBe('hand-off');
  });
});
