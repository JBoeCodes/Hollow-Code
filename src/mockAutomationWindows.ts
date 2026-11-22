// Browser-mock `automation_windows` / `set_automation_windows` (overnight
// Spare S10), kept in localStorage `mockAutomationWindows` so a reload sees
// the save. Refuses what `schedule_windows.rs` refuses, with its wording.
// `mockAutomationWindowsError` (localStorage) makes the next save fail with
// that message, for the error path.

import type { AutomationWindows, WindowDef, WindowSettings } from './generated/ipc';

const KEY = 'mockAutomationWindows';
const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri'];

export const MOCK_WINDOW_DEFAULTS: WindowSettings = {
  windows: [
    { name: 'tonight', label: 'Tonight (weekdays 18:00–07:00)', days: WEEKDAYS, start: '18:00', end: '07:00' },
    { name: 'weekday-evening', label: 'Weekday evenings (18:00–22:00)', days: WEEKDAYS, start: '18:00', end: '22:00' },
    { name: 'friday-pm', label: 'Friday afternoon (15:00–18:00)', days: ['fri'], start: '15:00', end: '18:00' },
    { name: 'nightly', label: 'Nightly (02:00–05:00)', days: [...WEEKDAYS, 'sat', 'sun'], start: '02:00', end: '05:00' },
  ],
  overnightCapUsd: 40,
};

const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const HM = /^([01]?\d|2[0-3]):[0-5]\d$/;

function problem(w: WindowDef): string | null {
  if (!w.name.trim() || !/^[a-z0-9-]+$/.test(w.name)) return `Window name '${w.name}' must be lowercase letters, digits and hyphens.`;
  if (w.days.length === 0 || w.days.some(d => !DAYS.includes(d.trim().toLowerCase().slice(0, 3)))) {
    return `Window '${w.name}' needs days from mon, tue, wed, thu, fri, sat, sun.`;
  }
  if (!HM.test(w.start.trim()) || !HM.test(w.end.trim())) return `Window '${w.name}' needs start and end as HH:MM.`;
  return null;
}

function validated(s: WindowSettings): WindowSettings {
  const seen = new Set<string>();
  for (const w of s.windows) {
    const p = problem(w);
    if (p) throw new Error(p);
    if (seen.has(w.name)) throw new Error(`Window '${w.name}' is defined twice.`);
    seen.add(w.name);
  }
  if (!Number.isFinite(s.overnightCapUsd) || s.overnightCapUsd < 0 || s.overnightCapUsd > 1000) {
    throw new Error('The overnight cap must be between $0 and $1,000.');
  }
  return s;
}

function stored(): WindowSettings {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? validated(JSON.parse(raw) as WindowSettings) : MOCK_WINDOW_DEFAULTS;
  } catch { return MOCK_WINDOW_DEFAULTS; }
}

function period(days: string[]): string {
  if (days.length === 1) return 'week';
  return days.length === 5 && WEEKDAYS.every(d => days.includes(d)) ? 'weekday' : 'day';
}

/** Fixed zone and no open window or next start, so smoke shots don't move. */
function view(s: WindowSettings): AutomationWindows {
  return {
    windows: s.windows.map(w => ({ ...w, period: period(w.days), openNow: false })),
    overnightCapUsd: s.overnightCapUsd,
    timeZone: localStorage.getItem('mockAutomationWindowsZone') || 'America/New_York',
  };
}

export function mockAutomationWindows(): AutomationWindows {
  if (localStorage.getItem('mockAutomationWindowsReadError')) throw new Error('Window settings unavailable');
  return view(stored());
}

export function mockSetAutomationWindows(args: Record<string, unknown> | undefined): AutomationWindows {
  const forced = localStorage.getItem('mockAutomationWindowsError');
  if (forced) { localStorage.removeItem('mockAutomationWindowsError'); throw new Error(forced); }
  const settings = validated((args?.settings ?? {}) as WindowSettings);
  localStorage.setItem(KEY, JSON.stringify(settings));
  return view(settings);
}
