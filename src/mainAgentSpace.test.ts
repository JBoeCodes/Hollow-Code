import { describe, expect, it } from 'vitest';
import work from './pages/Work.tsx?raw';
import body from './components/ChatBody.tsx?raw';
import menu from './cooMenu.ts?raw';
import wsMenu from './workspaceMenu.ts?raw';
import mainSettings from './settings/MainAgentSettings.tsx?raw';
import workflows from './pages/Workflows.tsx?raw';
import { mainAgentAttention, runningNightCount, checkInWords, checkInsOff, presence, spendGauge, observedProvider, recentActionsFact } from './mainAgentSpace';

describe('Main destination counts', () => {
  it('does not mount action history before the Main workspace exists', () => {
    expect(workflows).toContain('<Show when={allWorkspaces().some(isMainAgent)}>');
  });
  it('keeps unknown, failed, retained and empty action evidence distinct', () => {
    expect(recentActionsFact(0, 'loading')).toBe('Reading…');
    expect(recentActionsFact(0, 'error')).toBe('Unavailable');
    expect(recentActionsFact(0, 'ready')).toBe('0 recent');
    expect(recentActionsFact(3, 'ready')).toBe('3 recent');
    expect(recentActionsFact(3, 'error')).toBe('3 last read · refresh failed');
    expect(recentActionsFact(3, 'loading')).toBe('3 last read · refreshing');
  });
  it('the COO page has no header band; its menu actions live in the top bar menu (11-11)', () => {
    // The band (orb, status, day, More) took screen space and repeated what
    // the Subagents chip, Usage and Settings already show.
    expect(work).not.toContain('MainAgentSpace');
    expect(work).toContain('<ChatBody workspace />');
    expect(wsMenu).toContain('...cooMenuItems()');
    expect(menu).toContain('{ label: `Actions it took on its own · ${recentActionsFact(mainAgentActs().length, mainAgentActsRead())}`, action: () => void views().then(v => v.openMainAgentActions()) }');
    expect(menu).toContain("{ label: 'Change hard stop'");
    expect(menu).toContain("{ label: 'How check-ins run', action: () => void views().then(v => v.openBackgroundWork()) }");
    expect(menu).toContain("'Create default standing orders'");
    expect(menu).toContain("'Edit standing orders'");
    expect(menu).toContain("'Open failed conversation'");
  });
  it('its day is read in Settings → Background work → COO', () => {
    expect(mainSettings).toContain('checkInWords(st)');
    expect(mainSettings).toContain('todaySpendLine(st');
    expect(mainSettings).toContain('skippedWakeWords(st.wakeSkipped)');
  });
});

describe('Main hero evidence', () => {
  it('tints only a complete single-provider observation', () => {
    expect(observedProvider(['codex', 'codex'])).toBe('codex');
    expect(observedProvider(['claude', 'codex'])).toBeNull();
    expect(observedProvider(['claude', null])).toBeNull();
    expect(observedProvider([])).toBeNull();
    expect(observedProvider([''])).toBeNull();
  });
});

describe('the COO space', () => {
  it('does not promise an automatic tomorrow restart when the cap is zero', () => {
    const words = spendGauge({ spentTodayUsd: 0, dailyCapUsd: 0 }).words;
    expect(words).toContain('runs only when you ask');
    expect(words).not.toContain('tomorrow');
  });

  it('does not show a past check-in as the next scheduled time', () => {
    const now = new Date(2026, 9, 24, 10);
    const words = checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 24, 9, 30).toISOString(), checkInsToday: 2 }, now);
    expect(words).not.toContain('9:30');
    // Past due means it runs once something new comes up, not "no upcoming
    // check-in" (11-06, 3 PM on a working day).
    expect(words).toBe('2 check-ins today · next when something new comes up');
    expect(words).not.toContain('No upcoming');
    // Today's first is due regardless.
    expect(checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 24, 9, 30).toISOString(), checkInsToday: 0 }, now)).toBe('No check-ins yet today · the first is due now');
  });
  it('gauges spend against the hard stop: ok, warn from 75%, stop at the cap or a zero cap', () => {
    expect(spendGauge({ spentTodayUsd: 10, dailyCapUsd: 100 })).toEqual({ share: 0.1, tone: 'ok', words: '$10.00 of $100 on its own today' });
    expect(spendGauge({ spentTodayUsd: 80, dailyCapUsd: 100 }).tone).toBe('warn');
    const stop = spendGauge({ spentTodayUsd: 130, dailyCapUsd: 100 });
    expect(stop.share).toBe(1);
    expect(stop.tone).toBe('stop');
    expect(stop.words).toContain('stopped until tomorrow');
    expect(spendGauge({ spentTodayUsd: 0, dailyCapUsd: 0 }).tone).toBe('stop');
    expect(spendGauge({ spentTodayUsd: -1, dailyCapUsd: 100 }).share).toBe(0);
    // A cap with cents keeps them (a $2.50 cap is not "$3").
    expect(spendGauge({ spentTodayUsd: 1, dailyCapUsd: 2.5 }).words).toBe('$1.00 of $2.50 on its own today');
  });

  it('says when it next checks in', () => {
    const now = new Date(2026, 9, 10, 9, 0);
    expect(checkInWords(null, now)).toBe('Reading its day…');
    // Off is its own sentence; the page's button names the way to the setting (A07).
    expect(checkInWords({ checkIns: false, nextCheckIn: null, checkInsToday: 0 }, now)).toBe('Check-ins are off');
    // The count says what it counts: check-ins that ran today.
    expect(checkInWords({ checkIns: true, nextCheckIn: null, checkInsToday: 2 }, now)).toBe('2 check-ins today · none scheduled');
    expect(checkInWords({ checkIns: true, nextCheckIn: null, checkInsToday: 1 }, now)).toBe('1 check-in today · none scheduled');
    const first = checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 10, 9, 30).toISOString(), checkInsToday: 0 }, now);
    expect(first).toMatch(/^No check-ins yet today · first at 9:30/);
    // A later one runs only when something new came up.
    const later = checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 10, 9, 30).toISOString(), checkInsToday: 6 }, now);
    expect(later).toMatch(/^6 check-ins today · next 9:30.* if something new comes up$/);
    const monday = checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 12, 8, 0).toISOString(), checkInsToday: 0 }, now);
    expect(monday).toMatch(/^No check-ins today · next Mon 8:00/);
    expect(checkInWords({ checkIns: true, nextCheckIn: new Date(2026, 9, 12, 8, 0).toISOString(), checkInsToday: 12 }, now)).toMatch(/^12 check-ins today · next Mon 8:00/);
    expect(checkInWords({ checkIns: true, nextCheckIn: 'garbage', checkInsToday: 0 }, now)).toBe('No check-ins yet today · none scheduled');
  });

  it('counts only today’s check-ins and names the hard stop (review of #1182)', () => {
    const now = new Date(2026, 9, 13, 1, 0);
    // Read Monday 6:05 PM, still on screen after midnight: Monday's six are not today's.
    const monday = { checkIns: true, nextCheckIn: new Date(2026, 9, 13, 8, 5).toISOString(), checkInsToday: 6, lastCheckIn: new Date(2026, 9, 12, 17, 31).toISOString() };
    expect(checkInWords(monday, now)).toMatch(/^No check-ins yet today · first at 8:05/);
    expect(checkInWords({ ...monday, lastCheckIn: new Date(2026, 9, 13, 0, 30).toISOString() }, now)).toMatch(/^6 check-ins today · next 8:05.* if something new comes up$/);
    // At the hard stop nothing more runs today; a $0 stop, never on its own.
    const noon = new Date(2026, 9, 13, 12, 0);
    const today = { checkIns: true, nextCheckIn: new Date(2026, 9, 13, 11, 0).toISOString(), checkInsToday: 4, lastCheckIn: new Date(2026, 9, 13, 10, 30).toISOString() };
    expect(checkInWords({ ...today, spentTodayUsd: 100, dailyCapUsd: 100 }, noon)).toBe('4 check-ins today · no more today');
    expect(checkInWords({ ...today, checkInsToday: 0, lastCheckIn: null, spentTodayUsd: 100, dailyCapUsd: 100 }, noon)).toBe('No check-ins today');
    expect(checkInWords({ ...today, checkInsToday: 0, lastCheckIn: null, spentTodayUsd: 0, dailyCapUsd: 0 }, noon)).toBe('No check-ins while its hard stop is $0');
    expect(checkInWords({ ...today, spentTodayUsd: 0, dailyCapUsd: 0 }, noon)).toBe('4 check-ins today · none while its hard stop is $0');
    expect(checkInWords({ ...today, spentTodayUsd: 99, dailyCapUsd: 100 }, noon)).toBe('4 check-ins today · next when something new comes up');
    // The stop resets at midnight: Friday 5:55 PM at the stop, Monday's first still runs.
    const friday = new Date(2026, 9, 16, 17, 55);
    const weekend = { checkIns: true, nextCheckIn: new Date(2026, 9, 19, 8, 0).toISOString(), checkInsToday: 4, lastCheckIn: new Date(2026, 9, 16, 16, 0).toISOString(), spentTodayUsd: 100, dailyCapUsd: 100 };
    expect(checkInWords(weekend, friday)).toMatch(/^4 check-ins today · next Mon 8:00/);
  });

  it('presence: working (its turn or its chats), watching (check-ins on), else resting', () => {
    expect(presence(true, [], null)).toEqual({ state: 'working', words: 'Working' });
    expect(presence(true, [{ live: true }, { live: false }], null).words).toBe('Working · 1 subagent running');
    expect(presence(false, [{ live: true }, { live: true }], null)).toEqual({ state: 'working', words: '2 subagents running' });
    expect(presence(false, [{ live: false }], { checkIns: true }).state).toBe('watching');
    expect(presence(false, [], { checkIns: false }).state).toBe('resting');
    expect(presence(false, [], null).state).toBe('resting');
  });

  it('presence (asks 10-11 A2): waiting on you first, then working, then paused at its own cap', () => {
    expect(presence(true, [{ live: true }], { checkIns: true }, true)).toEqual({ state: 'waiting', words: 'Waiting on you · 1 subagent running' });
    expect(presence(false, [], null, true).words).toBe('Waiting on you');
    const capped = { checkIns: true, spentTodayUsd: 5, dailyCapUsd: 5 };
    expect(presence(false, [], capped).state).toBe('paused');
    expect(presence(false, [], { checkIns: true, spentTodayUsd: 0, dailyCapUsd: 0 }).state).toBe('paused');
    expect(presence(true, [], capped).state).toBe('working');
    expect(presence(false, [], { checkIns: true, spentTodayUsd: 4.99, dailyCapUsd: 5 }).state).toBe('watching');
  });

  it('the transcript lists no started chats; subagents are the chat header\'s chip and tab', () => {
    expect(body).not.toContain('MainAgentChildren');
    expect(body).toContain('<SidePanelCard />');
    expect(body).not.toContain('<SubagentsChip');
  });
});


it('counts own waiting conversations once and only running night rows', () => {
  expect(mainAgentAttention([{ key: 'own', state: 'waiting' }, { key: 'own', state: 'waiting' }, { key: 'other-own', state: 'failed' }])).toEqual({ waitingCount: 1, failed: true });
  expect(mainAgentAttention([])).toEqual({ waitingCount: 0, failed: false });
  // A chat Hollow's quit cut short is interrupted, not failed (P1-40).
  expect(mainAgentAttention([{ key: 'lane', state: 'interrupted' }])).toEqual({ waitingCount: 0, failed: false });
  expect(runningNightCount(null)).toBe(0);
  expect(runningNightCount([{ status: 'queued' }, { status: 'unknown' }, { status: 'done' }])).toBe(0);
  expect(runningNightCount([{ status: 'queued' }, { status: 'running', queueId: 1 }], [{ id: 1, status: 'running' }])).toBe(1);
  expect(presence(false, [], null, false, { nightRunning: 1 }).words).toBe('Tonight · 1 running');
  expect(presence(false, [], null, false, { failed: true }).state).toBe('failed');
});


it('keeps active work in failure words and places the waiting count beside its meaning', () => {
  expect(presence(true, [{ live: true }], null, false, { failed: true }).words).toBe('A conversation failed · Working · 1 subagent running');
  expect(presence(false, [{ live: true }], null, true, { waitingCount: 2 }).words).toBe('Waiting on you in 2 conversations · 1 subagent running');
  // The orb's badge number, in words beside what it counts.
  expect(presence(false, [], null, true, { waitingCount: 1 }).words).toBe('Waiting on you in 1 conversation');
});
it('does not turn a stale night snapshot or a queued turn into running work', () => {
  const item = { status: 'running', queueId: 7 };
  expect(runningNightCount([item], [{ id: 7, status: 'done' }])).toBe(0);
  expect(runningNightCount([item], [{ id: 7, status: 'queued' }])).toBe(0);
  expect(runningNightCount([item], [])).toBe(0);
  expect(runningNightCount([item], [{ id: 7, status: 'running' }])).toBe(1);
});

it('offers the check-ins setting only when its day is read and check-ins are off (A07)', () => {
  expect(checkInsOff(null)).toBe(false);
  expect(checkInsOff({ checkIns: true })).toBe(false);
  expect(checkInsOff({ checkIns: false })).toBe(true);
  // A menu item that lands on the switch itself, not a path written as text.
  expect(menu).toContain(`openSettingsFromOutside('automation', 'COO check-ins')`);
  expect(menu).toContain("checkInsOff(mainAgentStatus()) ? 'Turn check-ins on in Settings' : 'Check-in settings'");
  expect(menu).not.toContain('Settings → COO');
});
