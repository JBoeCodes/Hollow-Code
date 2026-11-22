import { describe, expect, it } from 'vitest';
import { foldKeepsText, foldRoles, foldSteps, foldSummary, foldTail, runFolds, type FoldableTurn, type FoldRole, type RunFold } from './runFold';

/** The per-index reference `foldRoles` must agree with. It shipped as an export until overnight H22; nothing but
 *  these tests read it, so it lives here as the oracle. */
function foldRole(folds: Map<number, RunFold>, turns: readonly { role: string }[], index: number): FoldRole {
  for (const f of folds.values()) {
    if (index < f.start || index > f.end) continue;
    if (turns[index]?.role !== 'assistant') return 'normal';
    const preface = (f.preface != null && f.preface !== f.answer && index === f.preface) || (f.opening != null && f.opening !== f.answer && index === f.opening);
    if (index === f.start) return f.answer === index || preface ? 'head-answer' : 'head';
    if (index === f.answer || preface) return 'answer';
    return 'hidden';
  }
  return 'normal';
}

const user = (text = 'Q'): FoldableTurn => ({ role: 'user', text, tools: [] });
const asst = (text: string, tools: string[] = [], extra: Partial<FoldableTurn> = {}): FoldableTurn =>
  ({ role: 'assistant', text, tools: tools.map(status => ({ status })), ...extra });
const named = (text: string, name: string): FoldableTurn =>
  ({ role: 'assistant', text, tools: [{ status: 'done', name }] });
const idle = { busy: false, liveRunId: null };

describe('finished-run fold (frontier F5)', () => {
  // The native shape: one agent message per tool call, then the answer.
  const turns = [
    user(),
    asst('Let me look.', ['done']),
    asst('', ['done']),
    asst('', ['error']),
    asst('Here is the answer.', [], { receipt: { startedAt: '2026-09-29T10:00:00Z', finishedAt: '2026-09-29T10:01:34Z' } }),
  ];

  it('folds a finished reply’s steps and narration above its answer', () => {
    const f = runFolds(turns, idle).get(1)!;
    expect(f).toMatchObject({ start: 1, end: 4, answer: 4, opening: 1, members: [1, 2, 3], steps: 3, failed: 1, durationMs: 94_000 });
    expect(foldSummary(f)).toBe('Worked for 1m 34s');
    expect(foldSteps(f)).toBe(' · 3 steps');
    expect(foldTail(f, turns)).toBe(4);
    const roles = foldRoles(runFolds(turns, idle), turns);
    // PER-13: "Let me look." was written before any tool ran, so it stays shown.
    expect([0, 1, 2, 3, 4].map(i => roles.get(i) ?? 'normal')).toEqual(['normal', 'head-answer', 'hidden', 'hidden', 'answer']);
  });

  it('keeps the words written before the first tool call visible (PER-13)', () => {
    // Opened with a tool call: nothing of it is kept.
    const toolFirst = [user(), asst('', ['done']), asst('Checked.', ['done']), asst('The answer.')];
    expect(runFolds(toolFirst, idle).get(1)?.opening).toBeNull();
    const roles = foldRoles(runFolds(toolFirst, idle), toolFirst);
    expect([1, 2, 3].map(i => roles.get(i))).toEqual(['head', 'hidden', 'answer']);
    // Words first on a turn of their own, then the work: kept, and not repeated in the fold.
    const wordsFirst = [user(), asst('I will check the logs first.'), asst('', ['done']), asst('Fixed.')];
    const f = runFolds(wordsFirst, idle).get(1)!;
    expect(f.opening).toBe(1);
    expect(foldKeepsText(f, 1)).toBe(true);
    expect(foldKeepsText(f, 3)).toBe(false);
    expect(foldRoles(runFolds(wordsFirst, idle), wordsFirst).get(1)).toBe('head-answer');
    // The opening that is the answer needs no second place.
    expect(runFolds([user(), asst('Done.', ['done'])], idle).get(1)?.opening).toBeNull();
  });

  it('keeps a live reply unfolded: streaming, carrying the live run, or busy at the end', () => {
    expect(runFolds(turns, { busy: true, liveRunId: null }).size).toBe(0);
    expect(runFolds([...turns.slice(0, 4), asst('Here', [], { partial: true })], idle).size).toBe(0);
    expect(runFolds([user(), asst('x', ['done'], { runId: 'r1' })], { busy: false, liveRunId: 'r1' }).size).toBe(0);
    // An earlier reply still folds while a later one runs.
    expect(runFolds([...turns, user(), asst('', ['running'])], { busy: true, liveRunId: null }).has(1)).toBe(true);
  });

  it('folds a one-turn reply’s tools above its own text, and skips replies with no tools', () => {
    const one = [user(), asst('Done.', ['done', 'done'])];
    const f = runFolds(one, idle).get(1)!;
    expect(f).toMatchObject({ answer: 1, members: [1], steps: 2, durationMs: null });
    expect(foldRoles(runFolds(one, idle), one).get(1)).toBe('head-answer');
    expect(foldSummary(f)).toBe('Worked · 2 steps');
    expect(foldSteps(f)).toBe('');
    expect(foldTail(f, one)).toBe(1);
    expect(runFolds([user(), asst('Just text.')], idle).size).toBe(0);
    // Turns with no message before them are not a reply.
    expect(runFolds([asst('', ['done']), asst('Done.')], idle).size).toBe(0);
  });

  it('leaves system notices in a folded reply visible, and handles a reply with no text', () => {
    const t = [user(), asst('', ['done']), { role: 'system', text: 'Stopped', tools: [] }];
    const folds = runFolds(t, idle);
    expect(folds.get(1)).toMatchObject({ answer: null, members: [1], steps: 1 });
    expect(foldRoles(folds, t).get(2) ?? 'normal').toBe('normal');
    // A system notice after the last assistant turn is not the tail.
    expect(foldTail(folds.get(1)!, t)).toBe(1);
    expect(foldSummary({ steps: 1, durationMs: 5_000 })).toBe('Worked for 5s');
    expect(foldSteps({ steps: 1, durationMs: 5_000 })).toBe(' · 1 step');
    expect(foldSummary({ steps: 3, durationMs: 70_000 })).toBe('Worked for 1m 10s');
    expect(foldSummary({ steps: 3, durationMs: 42_000 })).toBe('Worked for 42s');
    expect(foldSummary({ steps: 3, durationMs: 94_000, typicalMs: 240_000, workKind: 'code' })).toBe('Worked for 1m 34s · typically 4m 0s for code');
    expect(foldSummary({ steps: 3, durationMs: 94_000, typicalMs: null, workKind: 'code' })).toBe('Worked for 1m 34s');
  });

  it('times "Worked for" to the last step when the steps recorded one (overnight 11-12, C1)', () => {
    const receipt = { startedAt: '2026-11-11T10:00:00Z', finishedAt: '2026-11-11T10:03:00Z' };
    const step = (endedAt?: string) => ({ status: 'done', name: 'Read', endedAt });
    const live: FoldableTurn[] = [
      user(),
      { role: 'assistant', text: '', tools: [step('2026-11-11T10:00:30Z'), step('2026-11-11T10:01:10Z')] },
      { role: 'assistant', text: 'Answer.', tools: [], receipt },
    ];
    expect(runFolds(live, idle).get(1)!.durationMs).toBe(70_000);
    expect(foldSummary(runFolds(live, idle).get(1)!)).toBe('Worked for 1m 10s');
    // A reloaded transcript's steps carry no time: the receipt's finish.
    const saved = live.map(t => ({ ...t, tools: t.tools.map(x => ({ ...x, endedAt: undefined })) }));
    expect(runFolds(saved, idle).get(1)!.durationMs).toBe(180_000);
    // A step time before the start (another run's) is ignored.
    const odd = [live[0], { ...live[1], tools: [step('2026-11-11T09:00:00Z')] }, live[2]];
    expect(runFolds(odd, idle).get(1)!.durationMs).toBe(180_000);
  });

  it('keeps the text just before a question card visible when a later line is the answer', () => {
    const card = 'mcp__hollow_questions__ask_user_questions';
    const turns = [
      user(),
      asst('Let me look.', ['done']),
      asst('Which layout?'),
      named('', card),
      asst('Sent.'),
    ];
    const f = runFolds(turns, idle).get(1)!;
    expect(f).toMatchObject({ answer: 4, preface: 2, members: [1, 2, 3] });
    const roles = foldRoles(runFolds(turns, idle), turns);
    expect([0, 1, 2, 3, 4].map(i => roles.get(i) ?? 'normal')).toEqual(['normal', 'head-answer', 'answer', 'hidden', 'answer']);
    // The preface on the first assistant turn stays with the fold row.
    const headed = [user(), asst('Which layout?', ['done']), named('', card), asst('Sent.')];
    expect(runFolds(headed, idle).get(1)).toMatchObject({ answer: 3, preface: 1 });
    expect(foldRoles(runFolds(headed, idle), headed).get(1)).toBe('head-answer');
    // A fenced card counts, and a preface that is already the answer needs no second row.
    const fenced = [user(), asst('Which layout?\n```hollow-questions\n{}\n```', ['done'])];
    expect(runFolds(fenced, idle).get(1)).toMatchObject({ answer: 1, preface: null });
    // A reply with no card does not grow a preface.
    expect(runFolds([user(), asst('Look.', ['done']), asst('Done.')], idle).get(1)?.preface).toBeNull();
  });

  it('keeps the explanation when the card is only a hidden fence', () => {
    const fence = '```hollow-questions\n{"questions":[]}\n```';
    const split = [
      user(),
      asst('Let me look.', ['done']),
      asst('The page needs one layout.'),
      asst(fence),
    ];
    expect(runFolds(split, idle).get(1)).toMatchObject({ answer: 2, preface: null });
    const roles = foldRoles(runFolds(split, idle), split);
    expect([0, 1, 2, 3].map(i => roles.get(i) ?? 'normal')).toEqual(['normal', 'head-answer', 'answer', 'hidden']);
    const later = [...split, asst('Sent.')];
    expect(runFolds(later, idle).get(1)).toMatchObject({ answer: 4, preface: 2 });
    const nextOnly = [user(), asst('Look.', ['done']), asst('The answer.'), asst('```hollow-next\nNext\n```')];
    expect(runFolds(nextOnly, idle).get(1)).toMatchObject({ answer: 2, preface: null });
  });

  it('does not keep a preface when the question call was not posted', () => {
    const card = 'mcp__hollow_questions__ask_user_questions';
    const rejected = [
      user(),
      asst('Let me look.', ['done']),
      { role: 'assistant', text: '', tools: [{ status: 'error', name: card, isError: true, result: 'rejected' }] },
      asst('Done.'),
    ];
    expect(runFolds(rejected, idle).get(1)?.preface).toBeNull();
    const notPosted = [
      user(),
      asst('Let me look.', ['done']),
      { role: 'assistant', text: '', tools: [{ status: 'done', name: card, result: 'validation failed' }] },
      asst('Done.'),
    ];
    expect(runFolds(notPosted, idle).get(1)?.preface).toBeNull();
    const posted = [
      user(),
      asst('Let me look.', ['done']),
      { role: 'assistant', text: '', tools: [{ status: 'done', name: card, result: 'Posted a card. End your turn now.' }] },
      asst('Sent.'),
    ];
    expect(runFolds(posted, idle).get(1)).toMatchObject({ answer: 3, preface: 1 });
  });
});

// Overnight C5: one pass gives every row the role foldRole gives it, so a
// long thread's rows don't each loop over every fold.
describe('foldRoles (overnight C5)', () => {
  it('matches foldRole at every index of a long thread', () => {
    const sys: FoldableTurn = { role: 'system', text: 'notice', tools: [] };
    const reply = (n: number): FoldableTurn[] => [
      ...Array.from({ length: n }, (_, i) => asst(i % 2 ? '' : 'step', ['ok'])),
      ...(n % 3 === 0 ? [sys] : []),
      asst('The answer.'),
    ];
    const turns: FoldableTurn[] = [];
    for (let r = 0; r < 300; r++) turns.push(user(`Q${r}`), ...(r % 4 === 0 ? [asst('Plain reply.')] : reply(1 + (r % 5))));
    const folds = runFolds(turns, idle);
    expect(folds.size).toBeGreaterThan(100);
    const roles = foldRoles(folds, turns);
    for (let i = 0; i < turns.length; i++) expect(roles.get(i) ?? 'normal', `turn ${i}`).toBe(foldRole(folds, turns, i));
  });

  it('is what ChatBody reads per row', async () => {
    const chatBody = (await import('./components/ChatBody.tsx?raw')).default;
    expect(chatBody).toContain("const role = () => foldRoleOf().get(turnIdx()) ?? 'normal';");
    expect(chatBody).not.toContain('foldRole(folds(), chatTurns, turnIdx())');
    expect(chatBody).toContain('m !== f().answer && !foldKeepsText(f(), m) && member()?.text');
  });
});
