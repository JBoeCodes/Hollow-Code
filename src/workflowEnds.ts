// Automation end rules (G2): the "ends Fri 6 PM" text the list shows and the
// conversions the editor's end fields need. Pure (no store, Tauri or DOM), so
// the node vitest suite loads it. The Rust side owns the rule
// (`vault_workflows::end_if_due`); this only reads and writes its two keys.

/** The end-rule fields of a workflow, as the IPC type carries them. */
export interface EndFields {
  ends?: string | null;
  afterRuns?: number | null;
  runsCounted?: number | null;
  endedNote?: string | null;
}

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAY_MS = 86_400_000;

function clock(at: Date): string {
  const h = at.getHours();
  const m = at.getMinutes();
  const hour = h % 12 === 0 ? 12 : h % 12;
  return `${hour}${m ? `:${String(m).padStart(2, '0')}` : ''} ${h < 12 ? 'AM' : 'PM'}`;
}

/** "Fri 6 PM" within the coming week, "Nov 28, 6 PM" beyond it (local time). */
export function endsText(iso: string, now: Date): string | null {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return null;
  const soon = at.getTime() - now.getTime() < 6 * DAY_MS;
  const day = soon ? WEEKDAYS[at.getDay()] : `${MONTHS[at.getMonth()]} ${at.getDate()},`;
  return `${day} ${clock(at)}`;
}

/** What the list row says about an end rule: "ends Fri 6 PM", "ends after 3
 *  runs (1 done)", "ending now" (passed, not yet written), "bad end date" (a
 *  hand-written `ends:` that does not read) or "ended". Null for no rule. */
export function endsLabel(w: EndFields, now: Date = new Date()): string | null {
  if (w.endedNote) return 'ended';
  if (w.ends) {
    const at = new Date(w.ends);
    if (Number.isNaN(at.getTime())) return 'bad end date';
    // Past, but the scheduler has not written the end yet (it runs each minute).
    if (at.getTime() <= now.getTime()) return 'ending now';
    return `ends ${endsText(w.ends, now)}`;
  }
  if (w.afterRuns) {
    const done = w.runsCounted ?? 0;
    return `ends after ${w.afterRuns} ${w.afterRuns === 1 ? 'run' : 'runs'}${done ? ` (${done} done)` : ''}`;
  }
  return null;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** An ISO instant as the `datetime-local` input holds it (local time). */
export function toLocalInput(iso: string | null | undefined): string {
  if (!iso) return '';
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return '';
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}T${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/** The input's local date-time as an ISO instant; '' for empty, null for junk. */
export function fromLocalInput(local: string): string | null {
  if (!local.trim()) return '';
  const at = new Date(local);
  return Number.isNaN(at.getTime()) ? null : at.toISOString();
}

/** The end-rule keys a save sends: only what changed, so an untouched form
 *  (or an end time that has since passed) never re-sends or re-validates it.
 *  `ends: ''` and `afterRuns: 0` clear a rule. Picking one clears the other. */
export function endsForSave(
  ends: string, afterRuns: string, hydratedEnds: string, hydratedAfterRuns: string,
): { ends?: string; afterRuns?: number } {
  const out: { ends?: string; afterRuns?: number } = {};
  if (ends !== hydratedEnds) out.ends = fromLocalInput(ends) ?? ends;
  const runs = afterRuns.trim();
  if (runs !== hydratedAfterRuns) out.afterRuns = runs ? Math.max(0, Math.floor(Number(runs)) || 0) : 0;
  if (out.ends && out.afterRuns === undefined && hydratedAfterRuns) out.afterRuns = 0;
  if (out.afterRuns && out.ends === undefined && hydratedEnds) out.ends = '';
  return out;
}

/** The editor's one-line problem with the two fields, or null. */
export function endsProblem(ends: string, afterRuns: string, now: Date = new Date()): string | null {
  const runs = afterRuns.trim();
  if (ends && runs && Number(runs) > 0) return 'Pick an end time or a number of runs, not both.';
  if (ends) {
    const iso = fromLocalInput(ends);
    if (iso === null) return 'That end time is not a date and time.';
    if (new Date(iso).getTime() <= now.getTime()) return 'That end time has already passed.';
  }
  if (runs && !(Number.isInteger(Number(runs)) && Number(runs) >= 0)) return 'Runs is a whole number.';
  return null;
}
