/** Pure list logic for the themed Select (components/Select.tsx), kept free of
 *  Solid/DOM so it is unit-tested in the node vitest environment. */
export type SelectOption = {
  value: string;
  label: string;
  /** Secondary line: parent folder, branch, owner… (never the only cue). */
  detail?: string;
  /** Options sharing a group render under one heading, in first-seen order. */
  group?: string;
  disabled?: boolean;
  /** Extra search terms that are not displayed. */
  keywords?: string;
};

const fold = (s: string) => s.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase();

/** Every whitespace-separated term must appear in label, detail, group or
 *  keywords. Order is preserved; an empty query keeps everything. */
export function filterOptions(options: SelectOption[], query: string): SelectOption[] {
  const terms = fold(query).split(/\s+/).filter(Boolean);
  if (!terms.length) return options;
  return options.filter(o => {
    const hay = fold(`${o.label} ${o.detail ?? ''} ${o.group ?? ''} ${o.keywords ?? ''}`);
    return terms.every(t => hay.includes(t));
  });
}

/** Next enabled index stepping by ±1 from `from`, clamped (no wrap, like
 *  native menus). -1 when nothing is enabled in that direction. */
export function nextEnabled(options: SelectOption[], from: number, step: number): number {
  for (let i = from + step; i >= 0 && i < options.length; i += step) if (!options[i].disabled) return i;
  return from >= 0 && from < options.length && !options[from].disabled ? from : -1;
}

/** Typeahead: the first enabled option after `from` whose label starts with
 *  the typed prefix, wrapping. A repeated single letter cycles its matches. */
export function typeaheadMatch(options: SelectOption[], typed: string, from: number): number {
  const prefix = fold(typed);
  if (!prefix) return -1;
  const cycling = prefix.length > 1 && [...prefix].every(c => c === prefix[0]);
  const want = cycling ? prefix[0] : prefix;
  const n = options.length;
  const start = want.length === 1 ? from + 1 : Math.max(from, 0);
  for (let k = 0; k < n; k++) {
    const i = (start + k + n) % n;
    const o = options[i];
    if (!o.disabled && fold(o.label).startsWith(want)) return i;
  }
  return -1;
}

/** Contiguous runs by group, each item keeping its flat index for
 *  aria-activedescendant. */
export function groupOptions(options: SelectOption[]): { group?: string; items: { option: SelectOption; index: number }[] }[] {
  const out: { group?: string; items: { option: SelectOption; index: number }[] }[] = [];
  options.forEach((option, index) => {
    const last = out[out.length - 1];
    if (last && last.group === option.group) last.items.push({ option, index });
    else out.push({ group: option.group, items: [{ option, index }] });
  });
  return out;
}
