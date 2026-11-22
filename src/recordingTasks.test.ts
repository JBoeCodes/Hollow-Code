import { describe, expect, it } from 'vitest';
import { extractProposals } from './recordingTasks';

const summary = `## Overview
Discussed Q4 staffing.

## Action items
- Dana: Confirm the float supervisor budget with finance
- Draft the Q4 hiring plan (owner: Sam) by 2026-10-09
- Send the vendor questionnaire.
- Send the vendor questionnaire

## Decisions
- Keep two supervisors`;
const transcript = [
  { start: '00:05', end: '00:12', speaker: 'Dana', text: 'I will confirm the float supervisor budget with finance this week.' },
  { start: '03:10', end: '03:20', speaker: null, text: 'Someone should send the questionnaire to the vendors.' },
];

describe('recording task proposals (next-level P11)', () => {
  it('takes only listed actions, with explicit owners and dates and a timestamp', () => {
    const p = extractProposals('rec1', summary, transcript);
    expect(p.map(x => x.text)).toEqual(['Confirm the float supervisor budget with finance', 'Draft the Q4 hiring plan by 2026-10-09', 'Send the vendor questionnaire']);
    expect(p[0]).toMatchObject({ owner: 'Dana', due: null, at: '00:05' });
    expect(p[1]).toMatchObject({ owner: 'Sam', due: '2026-10-09' });
    expect(p[2].owner).toBeNull();
    expect(p.every(x => x.origin === 'meeting:rec1' && x.idempotencyKey.startsWith('recording:rec1\n'))).toBe(true);
  });
  it('a missing summary or transcript proposes nothing / no timestamp, never invents', () => {
    expect(extractProposals('r', null, transcript)).toEqual([]);
    expect(extractProposals('r', summary, null)[0].at).toBeNull();
    expect(extractProposals('r', 'Just notes.\n- Not an action', null)).toEqual([]);
  });
  it('the same recording and text give the same key (retries deduplicate)', () => {
    expect(extractProposals('r', summary, null)[0].idempotencyKey).toBe(extractProposals('r', summary, transcript)[0].idempotencyKey);
  });
  it('also opens at numbered and plain-line headings (durability D4)', () => {
    const forms = ['3. Action Items', '2) Next steps', 'Action items:', 'To do:', 'IV. Follow-ups:', '**Action points**', 'b) Tasks'];
    for (const head of forms) {
      const text = `Overview\nDiscussed Q4 staffing.\n\n${head}\n- Dana: Confirm the budget\n1. Send the vendor questionnaire`;
      expect(extractProposals('r', text, null).map(p => [p.text, p.owner]), head).toEqual([['Confirm the budget', 'Dana'], ['Send the vendor questionnaire', null]]);
    }
  });
  it('a numbered outline closes the actions section at its next heading', () => {
    const outline = '1. Overview\nDiscussed Q4 staffing.\n2. Decisions\n- Keep two supervisors\n3. Action Items\n1. Confirm the budget with finance\n2. Send the questionnaire.\n3. Follow up with Legal\n4. Risks\n- Budget may slip\n5. Open Questions:\n- Who owns hiring?';
    expect(extractProposals('r', outline, null).map(p => p.text)).toEqual(['Confirm the budget with finance', 'Send the questionnaire', 'Follow up with Legal']);
  });
  it('a plain "Risks:" heading closes it too', () => {
    const text = 'Action items:\n- Confirm the budget\nRisks:\n- Budget may slip';
    expect(extractProposals('r', text, null).map(p => p.text)).toEqual(['Confirm the budget']);
  });
  it('infers nothing from prose that talks about action items or what someone will do', () => {
    const prose = 'We went through the action items from last week. Dana will send the deck by Friday, and Sam will check the budget.\n2. We agreed the next steps can wait until October.\nAction items were not assigned in this meeting.';
    expect(extractProposals('r', prose, null)).toEqual([]);
  });
});

