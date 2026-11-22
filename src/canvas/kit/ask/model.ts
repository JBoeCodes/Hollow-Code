/** The ask kit's pure model (Canvas C4 A1, plan §5): option rows, answer
 *  names, each input's validation, the readable summary and the bridge
 *  `draft` payload. No DOM, so the node Vitest suite covers every rule the
 *  components (`components/*.ts`) apply; they only collect values and show
 *  the messages returned here.
 *
 *  **Answers.** Each field contributes one entry under its name: a string
 *  (`hc-choice`, `hc-text`, `hc-date`, `hc-pick`), a string array (`hc-multi`
 *  in authored order, `hc-rank` in the person's order), a number
 *  (`hc-range`), or a row → option object (`hc-matrix`). An unanswered
 *  optional field is `null`. The summary repeats the answered ones as
 *  `label: value` lines for a person to read; the agent reads `answers`. */

/** The bridge's limits for `draft` (`LIMITS.draftBytes`,
 *  `LIMITS.draftSummaryChars` in `src/canvasBridge.ts`; a test pins them
 *  equal). */
export const DRAFT_ANSWERS_BYTES = 16 * 1024;
export const DRAFT_SUMMARY_CHARS = 500;

/** A field's name: what the agent reads its answer under. */
export const FIELD_NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;

/** `hc-text` lengths (characters): the default for one line and for `long`,
 *  and the most an author may allow. */
export const TEXT_MAX = { short: 200, long: 2000, cap: 8000 } as const;

export type FieldKind = 'choice' | 'multi' | 'range' | 'rank' | 'text' | 'date' | 'matrix' | 'pick';
export type AnswerValue = string | number | string[] | { [row: string]: string } | null;

export interface Option { label: string; recommended: boolean }

/** Option rows: one per line, blank lines skipped, a leading `* ` marks the
 *  recommended option. A repeated label is kept once. */
export function parseOptions(lines: string[]): Option[] {
  const out: Option[] = [];
  const seen = new Set<string>();
  for (const raw of lines) {
    let t = raw.trim();
    const recommended = /^\*\s+/.test(t);
    if (recommended) t = t.replace(/^\*\s+/, '');
    if (!t || seen.has(t)) continue;
    seen.add(t);
    out.push({ label: t, recommended });
  }
  return out;
}

/** A short list attribute (`hc-matrix`'s `options`): `A | B` or `A, B`. */
export function splitList(s: string): string[] {
  const parts = s.includes('|') ? s.split('|') : s.split(',');
  return [...new Set(parts.map(p => p.trim()).filter(Boolean))];
}

/** The name an answer is filed under: the author's `name` when it is valid,
 *  else one made from the label (`Merge cadence` → `merge-cadence`), else
 *  `field-N`. A name already taken gets `-2`, `-3`… */
export function fieldName(name: string, label: string, index: number, taken: Set<string>): string {
  let base = name.trim();
  if (!FIELD_NAME.test(base)) {
    base = label.toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48);
    if (!FIELD_NAME.test(base)) base = `field-${index + 1}`;
  }
  let out = base;
  for (let n = 2; taken.has(out); n++) out = `${base}-${n}`;
  taken.add(out);
  return out;
}

/** A whole number attribute (counts, lengths), or null. */
export function intAttr(v: string | null | undefined): number | null {
  if (v == null || !/^\s*\d{1,6}\s*$/.test(v)) return null;
  return Number(v);
}

/** A number attribute (`min`, `max`, `step`, `value`), or null. */
export function numAttr(v: string | null | undefined): number | null {
  if (v == null || !/^\s*[+-]?(\d+\.?\d*|\.\d+)\s*$/.test(v)) return null;
  return Number(v);
}

export interface RangeSpec { min: number; max: number; step: number }

/** `hc-range`'s bounds: `min` 0 and `max` 100 by default, `step` 1; a
 *  reversed pair is swapped and a step ≤ 0 is 1. */
export function rangeSpec(min: string | null, max: string | null, step: string | null): RangeSpec {
  let lo = numAttr(min) ?? 0;
  let hi = numAttr(max) ?? 100;
  if (lo > hi) [lo, hi] = [hi, lo];
  const s = numAttr(step);
  return { min: lo, max: hi, step: s && s > 0 ? s : 1 };
}

/** Whether `v` is `min` plus a whole number of steps (to float precision). */
export function onStep(v: number, spec: RangeSpec): boolean {
  const k = (v - spec.min) / spec.step;
  return Math.abs(k - Math.round(k)) < 1e-6;
}

/** `hc-date`: an ISO calendar date that exists. */
export const ISO_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
export function isIsoDate(s: string): boolean {
  const m = ISO_DATE.exec(s);
  if (!m) return false;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
  return d.getUTCFullYear() === +m[1] && d.getUTCMonth() === +m[2] - 1 && d.getUTCDate() === +m[3];
}

/** What a field asks for, as its component reads it from its attributes. */
export interface FieldRule {
  kind: FieldKind;
  required: boolean;
  /** `hc-multi`: how many may be chosen. */
  minCount?: number | null;
  maxCount?: number | null;
  /** `hc-range`. */
  range?: RangeSpec;
  /** `hc-text`: the most characters allowed. */
  maxLength?: number;
  /** `hc-date`: ISO bounds ('' for none). */
  minDate?: string;
  maxDate?: string;
  /** `hc-matrix`: the row labels. */
  rows?: string[];
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many.replace('#', String(n)));

/** The message a field shows for `value`, or null when it is fine. The
 *  texts are short and say what to do. */
export function validate(rule: FieldRule, value: AnswerValue): string | null {
  const empty = value === null || value === '' || (Array.isArray(value) && value.length === 0);
  switch (rule.kind) {
    case 'choice':
      return rule.required && empty ? 'Choose one option.' : null;
    case 'pick':
      return rule.required && empty ? 'Pick one item.' : null;
    case 'multi': {
      const n = Array.isArray(value) ? value.length : 0;
      const min = Math.max(rule.minCount ?? 0, rule.required ? 1 : 0);
      if (n < min) return min === 1 ? 'Choose at least one option.' : `Choose at least ${min} options.`;
      if (rule.maxCount != null && n > rule.maxCount) return plural(rule.maxCount, 'Choose only one option.', 'Choose at most # options.');
      return null;
    }
    case 'range': {
      if (empty) return rule.required ? 'Set a value.' : null;
      const r = rule.range!;
      if (typeof value !== 'number' || !Number.isFinite(value) || value < r.min || value > r.max) return `Enter a number from ${r.min} to ${r.max}.`;
      return onStep(value, r) ? null : `Use steps of ${r.step} from ${r.min}.`;
    }
    case 'text': {
      const t = typeof value === 'string' ? value.trim() : '';
      if (!t) return rule.required ? 'Write an answer.' : null;
      const max = rule.maxLength ?? TEXT_MAX.short;
      return t.length > max ? `Keep it to ${max} characters.` : null;
    }
    case 'date': {
      if (empty) return rule.required ? 'Pick a date.' : null;
      if (typeof value !== 'string' || !isIsoDate(value)) return 'Enter a date as YYYY-MM-DD.';
      if (rule.minDate && value < rule.minDate) return `Pick a date on or after ${rule.minDate}.`;
      if (rule.maxDate && value > rule.maxDate) return `Pick a date on or before ${rule.maxDate}.`;
      return null;
    }
    case 'matrix': {
      if (!rule.required) return null;
      const got = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
      const left = (rule.rows ?? []).filter(r => !Object.prototype.hasOwnProperty.call(got, r)).length;
      return left ? `Answer every row (${left} left).` : null;
    }
    case 'rank':
      return null;
  }
}

/** One answered field as a person reads it, or null when it has no answer. */
export function display(kind: FieldKind, value: AnswerValue, unit = ''): string | null {
  if (value === null || value === '') return null;
  if (typeof value === 'number') return unit ? `${value} ${unit}` : String(value);
  if (typeof value === 'string') return value.replace(/\s+/g, ' ').trim() || null;
  if (Array.isArray(value)) {
    if (!value.length) return null;
    return kind === 'rank' ? value.map((v, i) => `${i + 1}. ${v}`).join('; ') : value.join(', ');
  }
  const rows = Object.keys(value).map(r => `${r}: ${value[r]}`);
  return rows.length ? rows.join('; ') : null;
}

export interface FieldAnswer { name: string; label: string; kind: FieldKind; value: AnswerValue; unit?: string }

/** The readable answer, one `label: value` line per answered field. */
export function summaryText(fields: FieldAnswer[]): string {
  const lines: string[] = [];
  for (const f of fields) {
    const v = display(f.kind, f.value, f.unit);
    if (v !== null) lines.push(`${f.label || f.name}: ${v}`);
  }
  return lines.length ? lines.join('\n') : 'No answers given.';
}

/** `s` cut to at most `max` UTF-16 units with `…`, never splitting a
 *  surrogate pair. */
export function clip(s: string, max: number): string {
  if (s.length <= max) return s;
  let cut = max - 1;
  const c = s.charCodeAt(cut - 1);
  if (c >= 0xd800 && c <= 0xdbff) cut -= 1;
  return `${s.slice(0, cut)}…`;
}

/** UTF-8 bytes of `value` as JSON. */
export function jsonBytes(value: unknown): number {
  return new TextEncoder().encode(JSON.stringify(value)).length;
}

export interface Draft { canvasId: string; answers: { [name: string]: AnswerValue }; summary: string }
export type DraftResult = { ok: true; draft: Draft } | { ok: false; error: 'too-large'; bytes: number };

/** Collect the fields into the bridge `draft` payload: `answers` by name
 *  (own data properties, so a name never reaches a prototype), at most
 *  `DRAFT_ANSWERS_BYTES` as UTF-8 JSON, and the summary clipped to
 *  `DRAFT_SUMMARY_CHARS`. */
export function buildDraft(canvasId: string, fields: FieldAnswer[]): DraftResult {
  const answers: { [name: string]: AnswerValue } = {};
  for (const f of fields) Object.defineProperty(answers, f.name, { value: f.value, enumerable: true, writable: true, configurable: true });
  const bytes = jsonBytes(answers);
  if (bytes > DRAFT_ANSWERS_BYTES) return { ok: false, error: 'too-large', bytes };
  return { ok: true, draft: { canvasId, answers, summary: clip(summaryText(fields), DRAFT_SUMMARY_CHARS) } };
}

export interface PickRow { label: string; cell: string; n: number | null }

/** `hc-pick`'s rows: `Label: 12` or `Label | 12` (the value as written in
 *  `cell`, its number in `n`), or just a label. Blank lines are skipped. */
export function pickRows(lines: string[]): PickRow[] {
  const out: PickRow[] = [];
  for (const raw of lines) {
    const t = raw.trim();
    if (!t) continue;
    const at = t.includes('|') ? t.indexOf('|') : t.lastIndexOf(':');
    const cell = at > 0 ? t.slice(at + 1).replace(/\|$/, '').trim() : '';
    const label = at > 0 && /\d/.test(cell) ? t.slice(0, at).trim() : t;
    const m = label === t ? null : /[+-]?(\d*\.?\d+)/.exec(cell.replace(/(\d),(?=\d{3}\b)/g, '$1'));
    out.push({ label, cell: label === t ? '' : cell, n: m ? Number(m[0]) : null });
  }
  return out;
}

/** Move `list[from]` to `to` (clamped); a new array. */
export function move<T>(list: T[], from: number, to: number): T[] {
  const out = list.slice();
  if (from < 0 || from >= out.length) return out;
  const [item] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(to, out.length)), 0, item);
  return out;
}

/** Where a rank key sends the item at `i` of `n`, or null for any other key. */
export function rankTarget(key: string, i: number, n: number): number | null {
  switch (key) {
    case 'ArrowUp': return Math.max(0, i - 1);
    case 'ArrowDown': return Math.min(n - 1, i + 1);
    case 'Home': return 0;
    case 'End': return n - 1;
    default: return null;
  }
}
