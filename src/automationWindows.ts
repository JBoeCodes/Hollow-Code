// Settings, Background work, Automation windows (overnight Spare S10): the pure
// model behind the editor (`settings/AutomationWindowsPanel.tsx`). The rules
// mirror `schedule_windows.rs` (`WindowDef::problem`,
// `WindowSettings::validated`) so a draft the editor accepts is one the
// backend accepts; the backend stays the authority and its refusal is mapped
// back onto the control it names (`placeBackendError`).

import type { AutomationWindows, WindowSettings } from './generated/ipc';

export const DAY_KEYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
export type DayKey = typeof DAY_KEYS[number];
export const DAY_NAMES: Record<DayKey, string> = {
  mon: 'Mon', tue: 'Tue', wed: 'Wed', thu: 'Thu', fri: 'Fri', sat: 'Sat', sun: 'Sun',
};
const DAY_PLURALS: Record<DayKey, string> = {
  mon: 'Mondays', tue: 'Tuesdays', wed: 'Wednesdays', thu: 'Thursdays', fri: 'Fridays', sat: 'Saturdays', sun: 'Sundays',
};

/** The windows Hollow's own work looks up by name (`schedule_windows::JOBS`,
 *  the night queue, `wake_me`): their hours and days are the person's, but
 *  the editor never renames or removes them, since that would quietly stop
 *  the work below. The backend itself accepts any valid set. */
export const BUILT_IN_USES: Record<string, string> = {
  'tonight': 'The night queue runs in it, and the overnight cap is counted per night of it.',
  'weekday-evening': 'For automations you put on it; Hollow runs nothing of its own here.',
  'friday-pm': 'Hollow drafts the Friday weekly update when it opens.',
  'nightly': 'Hollow’s nightly tidy-up scan, snapshot and friction count run in it.',
};
export const isBuiltIn = (name: string) => Object.prototype.hasOwnProperty.call(BUILT_IN_USES, name);

/** `night_queue::night_budget`: the cap is the lower of the setting and this. */
export const OVERNIGHT_HARD_STOP_USD = 40;
export const CAP_MAX_USD = 1000;

export interface WindowDraft {
  /** Stable row identity for the editor (a new window has no saved name yet). */
  key: string;
  name: string;
  /** The label without its "(hours)" suffix: what the person types. */
  title: string;
  days: DayKey[];
  start: string;
  end: string;
  builtIn: boolean;
  /** Saved already: its name is what automations say (`window:<name>`), so
   *  it is fixed; a new one's name follows its title until typed. */
  saved: boolean;
  nameTouched: boolean;
  /** The saved label, kept verbatim while the window is unchanged. */
  savedLabel: string;
  savedShape: string;
}

export interface WindowsDraft {
  windows: WindowDraft[];
  /** As typed; parsed on save. */
  cap: string;
}

export type WindowField = 'name' | 'days' | 'hours';
export interface DraftProblems {
  windows: Record<string, Partial<Record<WindowField, string>>>;
  cap?: string;
  /** A problem with no control of its own. */
  general?: string;
}

const HM = /^([01]?\d|2[0-3]):([0-5]\d)$/;
const NAME = /^[a-z0-9-]+$/;
/** Exactly the hours part Hollow composes (`labelFor`) or ships: an optional
 *  days part, then `HH:MM–HH:MM`, in a trailing parenthesis. Anything else in
 *  parentheses ("Standup (9:30 with Bob)") is the person's and stays. */
const DAY_ABBR = '(?:Mon|Tue|Wed|Thu|Fri|Sat|Sun)';
const DAYS_PART = `(?:weekdays|weekends|every day|(?:Mon|Tues|Wednes|Thurs|Fri|Satur|Sun)days|${DAY_ABBR}(?:, ${DAY_ABBR})*)`;
const HOURS_SUFFIX = new RegExp(`\\s*\\((?:${DAYS_PART} )?\\d{2}:\\d{2}–\\d{2}:\\d{2}\\)\\s*$`, 'i');

export function dayKey(raw: string): DayKey | null {
  const k = raw.trim().toLowerCase().slice(0, 3);
  return (DAY_KEYS as readonly string[]).includes(k) ? (k as DayKey) : null;
}

export function sortDays(days: readonly DayKey[]): DayKey[] {
  return DAY_KEYS.filter(d => days.includes(d));
}

export function splitLabel(label: string): string {
  return label.replace(HOURS_SUFFIX, '').trim();
}

/** "Weekdays", "Every day", "Weekends", "Fridays", or "Mon, Wed, Fri". */
export function daysText(days: readonly DayKey[]): string {
  const d = sortDays(days);
  if (d.length === 7) return 'Every day';
  if (d.length === 5 && d.join() === 'mon,tue,wed,thu,fri') return 'Weekdays';
  if (d.length === 2 && d.join() === 'sat,sun') return 'Weekends';
  if (d.length === 1) return DAY_PLURALS[d[0]];
  if (d.length === 0) return 'No days';
  return d.map(k => DAY_NAMES[k]).join(', ');
}

const minutes = (hm: string) => {
  const m = HM.exec(hm.trim());
  return m ? Number(m[1]) * 60 + Number(m[2]) : null;
};
const pad = (hm: string) => {
  const m = HM.exec(hm.trim());
  return m ? `${m[1].padStart(2, '0')}:${m[2]}` : hm.trim();
};

/** Whether the window runs past midnight: an end at or before the start
 *  means the next day (`WindowDef::end`). */
export function endsNextDay(start: string, end: string): boolean {
  const s = minutes(start), e = minutes(end);
  return s != null && e != null && e <= s;
}

/** "18:00–07:00 (next day)". */
export function hoursText(start: string, end: string): string {
  return `${pad(start)}–${pad(end)}${endsNextDay(start, end) ? ' (next day)' : ''}`;
}

/** "Weekdays 18:00–07:00 (next day)": one line for a window. */
export function windowSummary(w: Pick<WindowDraft, 'days' | 'start' | 'end'>): string {
  return `${daysText(w.days)} ${hoursText(w.start, w.end)}`;
}

/** A name from a title: "Lunch break" → `lunch-break`. */
export function slugify(title: string): string {
  return title.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40);
}

const shape = (w: Pick<WindowDraft, 'title' | 'days' | 'start' | 'end'>) =>
  JSON.stringify([w.title.trim(), sortDays(w.days), pad(w.start), pad(w.end)]);

export function draftFromView(view: Pick<AutomationWindows, 'windows' | 'overnightCapUsd'>): WindowsDraft {
  return {
    windows: view.windows.map(w => {
      const days = sortDays(w.days.map(dayKey).filter((d): d is DayKey => d != null));
      const title = splitLabel(w.label) || w.name;
      const row = { title, days, start: pad(w.start), end: pad(w.end) };
      return {
        key: `saved:${w.name}`, name: w.name, ...row,
        builtIn: isBuiltIn(w.name), saved: true, nameTouched: true,
        savedLabel: w.label, savedShape: shape(row),
      };
    }),
    cap: view.overnightCapUsd.toFixed(2),
  };
}

let seq = 0;
/** A new custom window: weekdays 09:00–17:00, with a name no other window has. */
export function newWindow(existing: readonly WindowDraft[]): WindowDraft {
  const taken = new Set(existing.map(w => w.name));
  let n = 1;
  let name = 'new-window';
  while (taken.has(name)) name = `new-window-${++n}`;
  return {
    key: `new:${++seq}`, name, title: n === 1 ? 'New window' : `New window ${n}`,
    days: ['mon', 'tue', 'wed', 'thu', 'fri'], start: '09:00', end: '17:00',
    builtIn: false, saved: false, nameTouched: false, savedLabel: '', savedShape: '',
  };
}

/** Apply a title edit; an unsaved window's untouched name follows it. */
export function withTitle(w: WindowDraft, title: string): WindowDraft {
  if (w.saved || w.nameTouched) return { ...w, title };
  return { ...w, title, name: slugify(title) || w.name };
}

export function toggleDay(w: WindowDraft, day: DayKey): WindowDraft {
  return { ...w, days: w.days.includes(day) ? w.days.filter(d => d !== day) : sortDays([...w.days, day]) };
}

/** The label to store: the saved one while nothing about the window changed,
 *  else its title with the hours composed again, so the plain-English name
 *  automations show never names old hours. */
export function labelFor(w: WindowDraft): string {
  if (w.saved && shape(w) === w.savedShape) return w.savedLabel;
  // No "(next day)" here: the label's own parentheses hold the hours, as the
  // shipped labels do ("Tonight (weekdays 18:00–07:00)").
  const days = daysText(w.days).replace(/^(Every day|Weekdays|Weekends)$/, x => x.toLowerCase());
  return `${w.title.trim() || w.name} (${days} ${pad(w.start)}–${pad(w.end)})`;
}

export function parseCap(raw: string): number | null {
  const t = raw.trim().replace(/^\$/, '').replace(/,/g, '');
  if (!/^\d*\.?\d+$/.test(t)) return null;
  const n = Number(t);
  return Number.isFinite(n) && n >= 0 && n <= CAP_MAX_USD ? n : null;
}

/** What the backend would refuse, per control, in plain words. */
export function draftProblems(d: WindowsDraft): DraftProblems {
  const out: DraftProblems = { windows: {} };
  const put = (key: string, field: WindowField, text: string) => {
    const row = (out.windows[key] ??= {});
    row[field] ??= text;
  };
  const seen = new Map<string, string>();
  for (const w of d.windows) {
    if (!w.name.trim() || !NAME.test(w.name)) put(w.key, 'name', 'Use lowercase letters, digits and hyphens, like lunch-break.');
    else if (seen.has(w.name)) put(w.key, 'name', `Another window is already called ${w.name}.`);
    else seen.set(w.name, w.key);
    if (w.days.length === 0) put(w.key, 'days', 'Pick at least one day.');
    if (minutes(w.start) == null || minutes(w.end) == null) put(w.key, 'hours', 'Start and end need times like 18:00.');
  }
  if (parseCap(d.cap) == null) out.cap = `Enter a cap from $0 to $${CAP_MAX_USD.toLocaleString('en-US')}.`;
  return out;
}

export function hasProblems(p: DraftProblems): boolean {
  return !!p.cap || !!p.general || Object.values(p.windows).some(r => Object.values(r).some(Boolean));
}

/** The `set_automation_windows` payload. Call only when `draftProblems` is empty. */
export function toSettings(d: WindowsDraft): WindowSettings {
  return {
    windows: d.windows.map(w => ({
      name: w.name.trim(), label: labelFor(w), days: sortDays(w.days), start: pad(w.start), end: pad(w.end),
    })),
    overnightCapUsd: parseCap(d.cap) ?? Number.NaN,
  };
}

/** Whether the draft differs from what is saved (`base`). */
export function isDirty(d: WindowsDraft, base: WindowsDraft | null): boolean {
  if (!base) return false;
  const key = (x: WindowsDraft) => JSON.stringify([
    x.windows.map(w => [w.name, w.title.trim(), sortDays(w.days), pad(w.start), pad(w.end)]),
    parseCap(x.cap) ?? x.cap.trim(),
  ]);
  return key(d) !== key(base);
}

/** Put the backend's refusal next to the control it is about
 *  (`schedule_windows.rs` names the window in quotes), or as a general one. */
export function placeBackendError(message: string, d: WindowsDraft): DraftProblems {
  const out: DraftProblems = { windows: {} };
  if (/overnight cap/i.test(message)) { out.cap = message; return out; }
  const m = /^Window(?: name)? '([^']*)'/.exec(message.trim());
  if (m) {
    const matches = d.windows.filter(w => w.name.trim() === m[1]);
    const row = matches[matches.length - 1];
    if (row) {
      const field: WindowField = /needs days/.test(message) ? 'days'
        : /HH:MM|start and end/.test(message) ? 'hours' : 'name';
      out.windows[row.key] = { [field]: message };
      return out;
    }
  }
  out.general = message;
  return out;
}

/** "$40", "$12.50". */
export function usd(n: number): string {
  return Number.isInteger(n) ? `$${n.toLocaleString('en-US')}` : `$${n.toFixed(2)}`;
}

/** What the cap means tonight (the lower of it and the $40 hard stop). */
export function capNote(raw: string): string {
  const n = parseCap(raw);
  if (n == null) return '';
  if (n === 0) return 'At $0 no night run starts.';
  if (n > OVERNIGHT_HARD_STOP_USD) return `Above the ${usd(OVERNIGHT_HARD_STOP_USD)} hard stop: a night still stops at ${usd(OVERNIGHT_HARD_STOP_USD)}.`;
  return `A night stops once its runs have cost ${usd(n)}.`;
}

/** The draft after a save of an earlier draft, keeping edits typed while the
 *  save ran: each row now saved (by name) takes its saved state, so a new
 *  window that was saved is no longer "new" and its name is fixed. Rows keep
 *  their keys, so nothing on screen is re-created. */
export function rebaseDraft(current: WindowsDraft, saved: WindowsDraft): WindowsDraft {
  const byName = new Map(saved.windows.map(w => [w.name, w]));
  return {
    ...current,
    windows: current.windows.map(w => {
      const s = byName.get(w.name.trim());
      return s ? { ...w, builtIn: s.builtIn, saved: true, nameTouched: true, savedLabel: s.savedLabel, savedShape: s.savedShape } : w;
    }),
  };
}

/** Which reads of `automation_windows` may still land: only the newest one,
 *  and none that started before the last save began or ended (its answer
 *  could predate the save and would put the old windows back). */
export function createLoadGate() {
  let ticket = 0;
  let newestLoad = 0;
  let lastSave = 0;
  return {
    load: () => (newestLoad = ++ticket),
    save: () => { lastSave = ++ticket; },
    accepts: (load: number) => load === newestLoad && load > lastSave,
  };
}

/** How many automations run on window `name` (`schedule: window:<name>[/…]`). */
export function windowUseCount(schedules: readonly string[], name: string): number {
  const re = /^window:\s*([a-z0-9-]+)\s*(?:\/.*)?$/;
  return schedules.filter(s => re.exec((s ?? '').trim())?.[1] === name).length;
}
