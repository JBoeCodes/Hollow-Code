// The Work page stays mounted behind the other pages (overnight 11-04 U4,
// App.tsx), so its conversation keeps its scroll, draft and running turn.
// This says whether an element of it is out of sight. It was part of
// workPanel.ts, which went with the workspace panel (COO rebuild L4).

/** Whether `el` is out of sight in a retained page: detached, or inside a
 *  hidden or inert ancestor (the hidden Work page). Used before a queued
 *  focus or a window-level key acts for a conversation, so a retained chat
 *  never takes focus or keys from the page on screen. */
export function retainedOffscreen(el: Element | null | undefined): boolean {
  return !el || !el.isConnected || !!el.closest('[hidden], [inert]');
}
