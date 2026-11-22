/** Modal-grade surfaces: anything that owns the keyboard while it's open. No
 *  global shortcut or native menu action may fire behind one; navigating out
 *  from under a form you're filling in is the shape of a lost edit. */
// A surface playing its exit animation (`.is-closing`, components/exit.ts)
// no longer owns the keyboard: an action clicked during the 150 ms fade was
// silently dropped (found by next-level-settings.spec, 2026-10-01).
// The workspace map (overnight B4) owns the keyboard too: no page shortcut or
// list key may act on the page hidden behind it. So does the Runbooks view
// (overnight A7), Mission Control (overnight 10-11 B2), the triage inbox
// (overnight 10-11 B6), the commitments ledger (overnight 10-11 A5), Ask
// your vault (overnight 10-11 A6), the weekly review (A7), the welcome tour
// and What's new (S1), and the canvas gallery (overnight 10-11/12 F3).
// Today, the weekly review, the map and Mission Control left the list in
// consolidation D (2026-10-13): they are views of Your work, not overlays.
// Ask your vault (`.av-backdrop`) and Ask Hollow left with One Ask
// (2026-11-03): every question is a chat.
// The commitments ledger, the canvas gallery and the Runbooks view left with
// consolidation lane B: Tasks → Commitments, Library → Results → Canvases and
// Automations → Runbooks / Tonight / Last night; the triage inbox became
// Bell → Needs you → See all → Incoming. `.rbk-backdrop` stays: the dialogs that share its look
// (Attach to issue, conflict copies, night dry run, prep packs)
// still use it.
export const MODAL_SEL = '.modal:not(.is-closing), .confirm-backdrop:not(.is-closing), .palette-backdrop:not(.is-closing), .onb-overlay-host, .rbk-backdrop:not(.is-closing), .welcome-backdrop:not(.is-closing), .wn-backdrop:not(.is-closing)';
/** MODAL_SEL as one class per surface, and whether a closing one counts.
 *  A part of another shape is matched as a selector (`rest`). */
const PARTS = MODAL_SEL.split(',').map(part => part.trim()).map(part => {
  const m = /^\.([\w-]+)(:not\(\.is-closing\))?$/.exec(part);
  return m ? { cls: m[1], skipClosing: !!m[2] } : { selector: part };
});
const REST = PARTS.flatMap(p => ('selector' in p && p.selector ? [p.selector] : [])).join(', ');
let live: { doc: Document; sets: [HTMLCollectionOf<Element>, boolean][] } | null = null;
/** Whether a modal-grade surface is open. It runs on every keystroke, three
 *  times (leader keys, list keys, a page's keys), so it reads live class
 *  collections, which the engine keeps current, instead of matching the
 *  selector list against every element: ~4 ms a call with 5,000 tasks drawn
 *  (overnight 10-11/12 P4). */
export const modalOpen = (): boolean => {
  if (typeof document === 'undefined') return false;
  if (live?.doc !== document) {
    live = { doc: document, sets: PARTS.flatMap(p => ('cls' in p && p.cls ? [[document.getElementsByClassName(p.cls), p.skipClosing] as [HTMLCollectionOf<Element>, boolean]] : [])) };
  }
  for (const [set, skipClosing] of live.sets) {
    for (let i = 0; i < set.length; i++) if (!skipClosing || !set[i].classList.contains('is-closing')) return true;
  }
  return !!REST && !!document.querySelector(REST);
};
