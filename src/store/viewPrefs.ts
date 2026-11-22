// Three per-machine view preferences (t3code parity 4.7, A17): how a diff
// reads (wrap long lines, line numbers) and where a web link opens. Each has
// one signal and one setter, the only writer of its localStorage key, like
// store/prefs.ts. The diff ones are applied as data attributes on the root
// (styles/build-changes.css), at load and on change, as text size is.
import { createSignal } from 'solid-js';

function read(key: string, fallback: boolean): boolean {
  try { const v = localStorage.getItem(key); return v === null ? fallback : v === '1'; } catch { return fallback; }
}
function write(key: string, on: boolean, fallback: boolean) {
  try { if (on === fallback) localStorage.removeItem(key); else localStorage.setItem(key, on ? '1' : '0'); } catch { /* memory holds it */ }
}

const WRAP_KEY = 'hollow.diff.wrap';
const NUMBERS_KEY = 'hollow.diff.numbers';
/** Wrap long diff lines (on by default); off scrolls each hunk sideways. */
export const [diffWrap, setDiffWrapSignal] = createSignal(read(WRAP_KEY, true));
/** Show old and new line numbers in a diff (on by default). */
export const [diffNumbers, setDiffNumbersSignal] = createSignal(read(NUMBERS_KEY, true));
function applyDiff() {
  if (typeof document === 'undefined') return;
  const root = document.documentElement;
  if (diffWrap()) delete root.dataset.diffWrap; else root.dataset.diffWrap = 'off';
  if (diffNumbers()) delete root.dataset.diffNumbers; else root.dataset.diffNumbers = 'off';
}
applyDiff();
export function setDiffWrap(on: boolean) { setDiffWrapSignal(on); write(WRAP_KEY, on, true); applyDiff(); }
export function setDiffNumbers(on: boolean) { setDiffNumbersSignal(on); write(NUMBERS_KEY, on, true); applyDiff(); }

const LINK_KEY = 'hollow.links.target';
export type LinkTarget = 'preview' | 'browser';
/** Where a web link in a reply opens: the Preview pane's web mode (default) or the Mac's default browser. ⌘ or Shift reverses it per click. */
export const [linkTarget, setLinkTargetSignal] = createSignal<LinkTarget>((() => {
  try { return localStorage.getItem(LINK_KEY) === 'browser' ? 'browser' : 'preview'; } catch { return 'preview'; }
})());
export function setLinkTarget(t: LinkTarget) {
  setLinkTargetSignal(t);
  try { if (t === 'preview') localStorage.removeItem(LINK_KEY); else localStorage.setItem(LINK_KEY, t); } catch { /* memory holds it */ }
}
