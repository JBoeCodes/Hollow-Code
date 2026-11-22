// Shared collators. `a.localeCompare(b, undefined, options)` builds a collator on every call; a sort calls it
// n log n times (profiling night 11-20: 10,000 titles took 204 ms that way and 14 ms through one collator in
// node's V8; the Library switch fell from 86 to 68 ms in Chromium; WebKit, which the app runs on, was not
// measured). Same ordering, built once.

/** Natural order, case and accents ignored: "Note 2" before "Note 10". The Library's order. */
export const naturalCompare = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' }).compare;

/** Digits compared as numbers, otherwise the default order: "ENG-9" before "ENG-10". The Tasks list's key order. */
export const keyCompare = new Intl.Collator(undefined, { numeric: true }).compare;
