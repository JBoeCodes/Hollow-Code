import { describe, expect, it } from 'vitest';
import { LEAD_MAX, splitLead } from './leadText';

const AGENTS_HINT =
  'Choose the agent and model Hollow uses across new chats, assistance, imports, background checks and automations. '
  + 'Individual conversations and automations can override these defaults. '
  + 'Some of these need a particular agent and stop instead of switching: see Runtime capabilities below. '
  + 'Below them: Chat (how new chats start), Accounts, Self-hosted models and Skills.';

describe('Settings help text cut after its first sentence (2026-11-07)', () => {
  it('keeps the first sentence and hands back every word after it', () => {
    const cut = splitLead(AGENTS_HINT);
    expect(cut).not.toBeNull();
    const [lead, rest] = cut!;
    expect(lead).toBe('Choose the agent and model Hollow uses across new chats, assistance, imports, background checks and automations.');
    expect(rest.startsWith('Individual conversations')).toBe(true);
    // Nothing is dropped: lead and rest together are the whole text.
    expect(`${lead} ${rest}`).toBe(AGENTS_HINT);
  });

  it('leaves a short text alone, and one with no sentence break to cut at', () => {
    expect(splitLead('Short help.')).toBeNull();
    expect(splitLead('x'.repeat(LEAD_MAX))).toBeNull();
    expect(splitLead('a'.repeat(200))).toBeNull();
    // One long sentence with no full stop and capital after it.
    expect(splitLead(`${'word '.repeat(60)}ends here.`)).toBeNull();
  });

  it('does not cut at a tiny first sentence, a lowercase continuation or a trailing stop', () => {
    const tiny = `Yes. ${'More words follow in the same unbroken sentence '.repeat(4)}and so on.`;
    expect(splitLead(tiny)).toBeNull();
    const lower = `${'Hollow reads the setting e.g. and then continues in lower case '.repeat(4)}end.`;
    expect(splitLead(lower)).toBeNull();
    // A final stop has nothing after it to put behind More: show the whole text.
    expect(splitLead(`${'word '.repeat(40)}done. `)).toBeNull();
  });

  it('does not cut after an abbreviation or a one or two letter word', () => {
    const text = 'Hollow uses an installed agent such as Claude, e.g. Claude Code, or the U.S. Gov build of it vs. Codex for every chat. '
      + 'Individual chats can override the choice. '
      + 'The rest is detail that only matters after you have looked at the first sentence once.';
    const [lead, rest] = splitLead(text)!;
    expect(lead).toBe('Hollow uses an installed agent such as Claude, e.g. Claude Code, or the U.S. Gov build of it vs. Codex for every chat.');
    expect(rest.startsWith('Individual chats')).toBe(true);
    // A real sentence that ends in a short word only keeps one more sentence in the lead.
    const short = 'Turn this on for every chat that should use the runner, so builds and tests stay off this Mac whenever it is busy, and leave it on. '
      + 'Then the rest follows here and it is long enough to need a fold. Last sentence.';
    const [shortLead] = splitLead(short)!;
    expect(shortLead.endsWith('Then the rest follows here and it is long enough to need a fold.')).toBe(true);
  });

  it('cuts at the first break past the minimum, not an earlier tiny one', () => {
    const text = `Yes. ${'This is the real first sentence and it runs a good while longer than the minimum. '}${'Then the rest follows. '.repeat(5)}`;
    const [lead] = splitLead(text)!;
    expect(lead).toBe('Yes. This is the real first sentence and it runs a good while longer than the minimum.');
  });

  it('takes a custom length', () => {
    expect(splitLead('First sentence is long enough to be a lead here. Second sentence follows it.', 20)).toEqual([
      'First sentence is long enough to be a lead here.',
      'Second sentence follows it.',
    ]);
  });
});
