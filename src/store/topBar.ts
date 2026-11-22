import { createSignal } from 'solid-js';

/** Where a page may put its own controls in the top bar (2026-09-29): the
 *  open workspace puts its live state and branch chip there, right after its
 *  name in the breadcrumb (asks 10-11 A3), so it needs no header line of its
 *  own. Set by TopBar. */
export const [topBarSlot, setTopBarSlot] = createSignal<HTMLElement>();
/** The top bar's far-right edge (asks 10-11 A3): the open workspace's
 *  side-panel toggle, mirroring the sidebar toggle at the far left. */
export const [topBarPaneSlot, setTopBarPaneSlot] = createSignal<HTMLElement>();
