import { describe, expect, it } from 'vitest';
import { endsForSave, endsLabel, endsProblem, endsText, fromLocalInput, toLocalInput } from './workflowEnds';

// Local-time constructors keep these tests the same in every time zone.
const now = new Date(2026, 10, 15, 9, 0); // Sun Nov 15 2026, 9:00 AM

describe('endsText and endsLabel', () => {
  it('writes a weekday and hour within the week', () => {
    const friday = new Date(2026, 10, 20, 18, 0).toISOString();
    expect(endsText(friday, now)).toBe('Fri 6 PM');
    expect(endsLabel({ ends: friday }, now)).toBe('ends Fri 6 PM');
    expect(endsLabel({ ends: new Date(2026, 10, 20, 18, 30).toISOString() }, now)).toBe('ends Fri 6:30 PM');
    expect(endsText(new Date(2026, 10, 16, 0, 0).toISOString(), now)).toBe('Mon 12 AM');
  });

  it('writes the date once it is a week or more away', () => {
    expect(endsLabel({ ends: new Date(2026, 10, 28, 6, 0).toISOString() }, now)).toBe('ends Nov 28, 6 AM');
  });

  it('counts runs, and says ended once stopped', () => {
    expect(endsLabel({ afterRuns: 3 }, now)).toBe('ends after 3 runs');
    expect(endsLabel({ afterRuns: 1 }, now)).toBe('ends after 1 run');
    expect(endsLabel({ afterRuns: 3, runsCounted: 2 }, now)).toBe('ends after 3 runs (2 done)');
    expect(endsLabel({ endedNote: 'Ended 2026-11-14 because its end time passed.' }, now)).toBe('ended');
  });

  it('says nothing for no rule, and flags a passed or unreadable time', () => {
    expect(endsLabel({}, now)).toBeNull();
    expect(endsLabel({ ends: 'soon' }, now)).toBe('bad end date');
    expect(endsLabel({ ends: new Date(2026, 10, 15, 8, 0).toISOString() }, now)).toBe('ending now');
  });
});

describe('the editor conversions', () => {
  it('round-trips a local date-time through the input', () => {
    const iso = new Date(2026, 10, 20, 18, 5).toISOString();
    expect(toLocalInput(iso)).toBe('2026-11-20T18:05');
    expect(fromLocalInput('2026-11-20T18:05')).toBe(iso);
    expect(fromLocalInput('')).toBe('');
    expect(fromLocalInput('nope')).toBeNull();
    expect(toLocalInput(null)).toBe('');
  });

  it('sends only what changed, and one rule replaces the other', () => {
    expect(endsForSave('', '', '', '')).toEqual({});
    expect(endsForSave('2026-11-20T18:00', '', '2026-11-20T18:00', '')).toEqual({});
    expect(endsForSave('2026-11-20T18:00', '', '', '')).toEqual({ ends: new Date(2026, 10, 20, 18, 0).toISOString() });
    expect(endsForSave('', '', '2026-11-20T18:00', '')).toEqual({ ends: '' });
    expect(endsForSave('', '5', '', '')).toEqual({ afterRuns: 5 });
    expect(endsForSave('', '', '', '5')).toEqual({ afterRuns: 0 });
    expect(endsForSave('', '4', '2026-11-20T18:00', '')).toEqual({ afterRuns: 4, ends: '' });
    expect(endsForSave('2026-11-21T18:00', '', '', '4')).toEqual({ ends: new Date(2026, 10, 21, 18, 0).toISOString(), afterRuns: 0 });
  });

  it('names the problems the save would refuse', () => {
    expect(endsProblem('', '', now)).toBeNull();
    expect(endsProblem('2026-11-20T18:00', '', now)).toBeNull();
    expect(endsProblem('2026-11-14T18:00', '', now)).toMatch(/already passed/);
    expect(endsProblem('2026-11-20T18:00', '3', now)).toMatch(/not both/);
    expect(endsProblem('', '2.5', now)).toMatch(/whole number/);
  });
});
