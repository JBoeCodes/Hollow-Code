import { describe, expect, it, vi } from 'vitest';
import {
  capNote, createLoadGate, daysText, draftFromView, draftProblems, hasProblems, hoursText, isDirty, labelFor, newWindow, parseCap,
  placeBackendError, rebaseDraft, slugify, splitLabel, windowUseCount, toSettings, toggleDay, withTitle, type WindowDraft, type WindowsDraft,
} from './automationWindows';
import { MOCK_WINDOW_DEFAULTS, mockAutomationWindows, mockSetAutomationWindows } from './mockAutomationWindows';
import { describeSchedule } from './workflowSchedule';
import { rememberWindowLabels } from './windowLabels';

const view = () => ({
  overnightCapUsd: 40,
  windows: MOCK_WINDOW_DEFAULTS.windows.map(w => ({ ...w, period: 'day', openNow: false })),
});
const fresh = (): WindowsDraft => draftFromView(view());
const row = (d: WindowsDraft, name: string) => d.windows.find(w => w.name === name)!;
const withRow = (d: WindowsDraft, name: string, change: (w: WindowDraft) => WindowDraft) =>
  ({ ...d, windows: d.windows.map(w => (w.name === name ? change(w) : w)) });

describe('automation windows editor model (overnight S10)', () => {
  it('reads the saved windows: built-ins fixed by name, titles without their hours', () => {
    const d = fresh();
    expect(d.windows.map(w => [w.name, w.title, w.builtIn])).toEqual([
      ['tonight', 'Tonight', true], ['weekday-evening', 'Weekday evenings', true],
      ['friday-pm', 'Friday afternoon', true], ['nightly', 'Nightly', true],
    ]);
    expect(d.cap).toBe('40.00');
    expect(splitLabel('Lunch (weekdays 12:00–13:00)')).toBe('Lunch');
    expect(splitLabel('Lunch (Mon, Wed 12:00–13:00)')).toBe('Lunch');
    expect(splitLabel('Friday afternoon (Fridays 14:00–18:00)')).toBe('Friday afternoon');
    expect(splitLabel('Quiet hours (no time here)')).toBe('Quiet hours (no time here)');
  });

  it('strips only the hours part Hollow composes, never the person\'s own parentheses', () => {
    expect(splitLabel('Standup (9:30 with Bob)')).toBe('Standup (9:30 with Bob)');
    expect(splitLabel('Standup (9:30 with Bob) (weekdays 09:30–09:45)')).toBe('Standup (9:30 with Bob)');
    expect(splitLabel('Call (until 18:00–19:00 maybe)')).toBe('Call (until 18:00–19:00 maybe)');
    const d = draftFromView({ overnightCapUsd: 40, windows: [{ name: 'standup', label: 'Standup (9:30 with Bob)', days: ['mon'], start: '09:30', end: '09:45', period: 'week', openNow: false }] });
    expect(d.windows[0].title).toBe('Standup (9:30 with Bob)');
    const moved = { ...d.windows[0], start: '10:00' };
    expect(labelFor(moved)).toBe('Standup (9:30 with Bob) (Mondays 10:00–09:45)');
    expect(splitLabel(labelFor(moved))).toBe('Standup (9:30 with Bob)');
  });

  it('round-trips unchanged windows byte for byte', () => {
    const d = fresh();
    expect(isDirty(d, fresh())).toBe(false);
    expect(toSettings(d)).toEqual(MOCK_WINDOW_DEFAULTS);
  });

  it('describes days and hours plainly, with the next-day end', () => {
    expect(daysText(['mon', 'tue', 'wed', 'thu', 'fri'])).toBe('Weekdays');
    expect(daysText(['sun', 'sat'])).toBe('Weekends');
    expect(daysText(['fri'])).toBe('Fridays');
    expect(daysText(['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'])).toBe('Every day');
    expect(daysText(['wed', 'mon'])).toBe('Mon, Wed');
    expect(hoursText('18:00', '07:00')).toBe('18:00–07:00 (next day)');
    expect(hoursText('9:00', '17:00')).toBe('09:00–17:00');
    expect(hoursText('02:00', '02:00')).toBe('02:00–02:00 (next day)');
  });

  it('composes the label again once hours or days change, so automations never name old hours', () => {
    const d = withRow(fresh(), 'friday-pm', w => ({ ...w, start: '14:00' }));
    expect(isDirty(d, fresh())).toBe(true);
    expect(labelFor(row(d, 'friday-pm'))).toBe('Friday afternoon (Fridays 14:00–18:00)');
    const t = withRow(fresh(), 'tonight', w => toggleDay(w, 'fri'));
    expect(labelFor(row(t, 'tonight'))).toBe('Tonight (Mon, Tue, Wed, Thu 18:00–07:00)');
    expect(labelFor(row(fresh(), 'nightly'))).toBe('Nightly (02:00–05:00)');
  });

  it('adds a custom window whose name follows its title until typed, and never collides', () => {
    const d = fresh();
    const a = newWindow(d.windows);
    expect(a).toMatchObject({ name: 'new-window', builtIn: false, saved: false, days: ['mon', 'tue', 'wed', 'thu', 'fri'] });
    expect(newWindow([...d.windows, a]).name).toBe('new-window-2');
    const titled = withTitle(a, 'Lunch break!');
    expect(titled.name).toBe('lunch-break');
    const typed = withTitle({ ...titled, name: 'lunch', nameTouched: true }, 'Long lunch');
    expect(typed.name).toBe('lunch');
    // A saved window keeps its name whatever the title says.
    expect(withTitle(row(d, 'nightly'), 'Small hours').name).toBe('nightly');
    expect(labelFor(titled)).toBe('Lunch break! (weekdays 09:00–17:00)');
    expect(slugify('Mañana café')).toBe('manana-cafe');
  });

  it('finds what the backend would refuse, per control', () => {
    let d = fresh();
    const extra = { ...newWindow(d.windows), name: 'nightly', nameTouched: true };
    d = { ...d, windows: [...d.windows, extra], cap: '1001' };
    d = withRow(d, 'friday-pm', w => ({ ...w, days: [], end: '' }));
    const p = draftProblems(d);
    expect(hasProblems(p)).toBe(true);
    expect(p.windows[extra.key]?.name).toBe('Another window is already called nightly.');
    expect(p.windows[row(d, 'friday-pm').key]).toEqual({ days: 'Pick at least one day.', hours: 'Start and end need times like 18:00.' });
    expect(p.cap).toBe('Enter a cap from $0 to $1,000.');
    expect(draftProblems({ ...fresh(), windows: [{ ...extra, name: 'Lunch Break' }] }).windows[extra.key]?.name).toMatch(/lowercase/);
    expect(hasProblems(draftProblems(fresh()))).toBe(false);
  });

  it('reads the cap as dollars from 0 to 1,000 and says what it means tonight', () => {
    expect(parseCap('40')).toBe(40);
    expect(parseCap('$12.50')).toBe(12.5);
    expect(parseCap('1,000')).toBe(1000);
    expect(parseCap('.5')).toBe(0.5);
    expect(parseCap('40.00')).toBe(40);
    for (const bad of ['', '-1', '1000.01', 'abc', 'NaN']) expect(parseCap(bad), bad).toBeNull();
    expect(capNote('0')).toMatch(/no night run starts/);
    expect(capNote('25')).toBe('A night stops once its runs have cost $25.');
    expect(capNote('100')).toMatch(/still stops at \$40/);
  });

  it('puts a backend refusal next to the control it names, else as a general one', () => {
    const d = fresh();
    const key = row(d, 'nightly').key;
    expect(placeBackendError("Window 'nightly' needs start and end as HH:MM.", d).windows[key]).toEqual({ hours: "Window 'nightly' needs start and end as HH:MM." });
    expect(placeBackendError("Window 'nightly' needs days from mon, tue, wed, thu, fri, sat, sun.", d).windows[key]).toHaveProperty('days');
    expect(placeBackendError("Window 'nightly' is defined twice.", d).windows[key]).toHaveProperty('name');
    expect(placeBackendError('The overnight cap must be between $0 and $1,000.', d).cap).toMatch(/overnight cap/);
    expect(placeBackendError('scheduler write: Permission denied', d)).toEqual({ windows: {}, general: 'scheduler write: Permission denied' });
  });

  it('keeps edits typed during a save, and takes the saved state for what was saved', () => {
    const before = fresh();
    const added = withTitle(newWindow(before.windows), 'Lunch');
    const sent = { ...before, windows: [...before.windows, added] };
    // While the save ran: the person moved nightly and retitled the new row.
    const typed = { ...sent, windows: sent.windows.map(w => w.name === 'nightly' ? { ...w, end: '06:00' } : w.key === added.key ? { ...w, title: 'Long lunch' } : w) };
    const savedView = { overnightCapUsd: 40, windows: toSettings(sent).windows.map(w => ({ ...w, period: 'day', openNow: false })) };
    const savedDraft = draftFromView(savedView);
    const next = rebaseDraft(typed, savedDraft);
    expect(row(next, 'nightly').end).toBe('06:00');
    expect(next.windows.find(w => w.key === added.key)).toMatchObject({ title: 'Long lunch', name: 'lunch', saved: true, nameTouched: true });
    expect(next.windows.map(w => w.key)).toEqual(typed.windows.map(w => w.key));
    // Still dirty against the saved base, for exactly the newer edits.
    expect(isDirty(next, savedDraft)).toBe(true);
    expect(isDirty(rebaseDraft(sent, savedDraft), savedDraft)).toBe(false);
  });

  it('lets only the newest read land, and none that started before a save', () => {
    const gate = createLoadGate();
    const first = gate.load();
    const second = gate.load();
    expect(gate.accepts(first)).toBe(false);
    expect(gate.accepts(second)).toBe(true);
    const during = gate.load();
    gate.save(); // the save starts after this read started
    expect(gate.accepts(during)).toBe(false);
    const inSave = gate.load();
    gate.save(); // and ends after this one started
    expect(gate.accepts(inSave)).toBe(false);
    expect(gate.accepts(gate.load())).toBe(true);
  });

  it('counts the automations on a window, catch-up suffix or not', () => {
    const schedules = ['window:lunch', 'window:lunch/run-all', 'window:lunch-break', '0 0 7 * * * *', 'manual', ''];
    expect(windowUseCount(schedules, 'lunch')).toBe(2);
    expect(windowUseCount(schedules, 'lunch-break')).toBe(1);
    expect(windowUseCount(schedules, 'nightly')).toBe(0);
  });

  it('the browser mock serves the shipped defaults and refuses as the backend does', () => {
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), removeItem: (k: string) => void store.delete(k) });
    expect(() => mockSetAutomationWindows({ settings: { ...MOCK_WINDOW_DEFAULTS, overnightCapUsd: 2000 } })).toThrow('The overnight cap must be between $0 and $1,000.');
    expect(store.size).toBe(0);
    expect(mockAutomationWindows().windows.map(w => [w.name, w.start, w.end])).toEqual([
      ['tonight', '18:00', '07:00'], ['weekday-evening', '18:00', '22:00'], ['friday-pm', '15:00', '18:00'], ['nightly', '02:00', '05:00'],
    ]);
    vi.unstubAllGlobals();
  });

  it('names a window schedule with its saved label once one was read', () => {
    expect(describeSchedule('window:lunch')).toBe('Window: lunch');
    rememberWindowLabels([{ name: 'lunch', label: 'Lunch (weekdays 12:00–13:00)' }, { name: 'friday-pm', label: 'Friday afternoon (Fridays 14:00–18:00)' }]);
    expect(describeSchedule('window:lunch')).toBe('Lunch (weekdays 12:00–13:00)');
    expect(describeSchedule('window:friday-pm/run-all')).toBe('Friday afternoon (Fridays 14:00–18:00) · catches up every missed run');
    rememberWindowLabels([]);
    expect(describeSchedule('window:friday-pm')).toBe('Friday afternoon (15:00–18:00)');
  });
});
