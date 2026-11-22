import { describe, expect, it } from 'vitest';
import { builtInCaps, capState, parseCapInput } from './builtInCaps';

/** P1-07: the caps Hollow keeps for you, listed above your own budgets. */
describe('built-in caps', () => {
  const full = {
    main: { spentUsd: 12.4, capUsd: 100 },
    overnightCapUsd: 40,
  };

  it('lists the COO hard stop and the overnight cap with their limits', () => {
    const rows = builtInCaps(full);
    expect(rows.map(r => r.id)).toEqual(['main-agent', 'overnight']);
    expect(rows.map(r => r.label)).toEqual(['COO daily hard stop', 'Overnight cap']);
    expect(rows[0]).toMatchObject({ spentUsd: 12.4, limitUsd: 100, editable: true, state: 'ok' });
    expect(rows[0].spentWords).toBe('$12.40 of $100 today');
    expect(rows[1]).toMatchObject({ spentUsd: null, limitUsd: 40, editable: true, state: 'ok', meterPercent: null });
  });

  it('keeps a row whose spend could not be read, saying so', () => {
    const rows = builtInCaps({ main: { spentUsd: null, capUsd: 100 }, overnightCapUsd: 40 });
    expect(rows.map(r => r.id)).toEqual(['main-agent', 'overnight']);
    expect(rows[0]).toMatchObject({ state: 'unknown', spentWords: 'Spend today not read yet' });
  });

  it('a $0 overnight cap reads as stopped', () => {
    expect(builtInCaps({ ...full, overnightCapUsd: 0 })[1]).toMatchObject({ state: 'stopped', limitUsd: 0 });
  });

  it('leaves out a cap whose limit is unknown only when it is editable and unread', () => {
    const rows = builtInCaps({ main: null, overnightCapUsd: null });
    expect(rows.find(r => r.id === 'main-agent')).toMatchObject({ limitUsd: null, editable: false });
    expect(rows.find(r => r.id === 'overnight')).toMatchObject({ limitUsd: null, editable: false });
  });

  it('names the state: warn from 75%, stopped at the cap, and a $0 cap stops everything', () => {
    expect(capState(10, 100)).toBe('ok');
    expect(capState(75, 100)).toBe('warn');
    expect(capState(100, 100)).toBe('stopped');
    expect(capState(0, 0)).toBe('stopped');
    expect(capState(null, 100)).toBe('unknown');
  });

  it('reads a typed cap in dollars', () => {
    expect(parseCapInput('25')).toBe(25);
    expect(parseCapInput(' $12.50 ')).toBe(12.5);
    expect(parseCapInput('0')).toBe(0);
    expect(parseCapInput('')).toBeNull();
    expect(parseCapInput('-3')).toBeNull();
    expect(parseCapInput('lots')).toBeNull();
    expect(parseCapInput('5000')).toBeNull();
  });

  it('does not turn unread spend into a measured percentage', () => {
    const rows = builtInCaps({ main: null, overnightCapUsd: 40 });
    expect(rows.map(row => row.meterPercent)).toEqual([null, null]);
  });

  it('clamps measured fill at the cap and leaves a zero cap unmeasured', () => {
    expect(builtInCaps({ ...full, main: { spentUsd: 150, capUsd: 100 } })[0]).toMatchObject({ state: 'stopped', meterPercent: 100 });
    expect(builtInCaps({ ...full, main: { spentUsd: 0, capUsd: 0 } })[0]).toMatchObject({ state: 'stopped', meterPercent: null });
  });
});
