import { createSignal, onCleanup } from 'solid-js';
import type { AgentSessionSummary } from './types';
import type { ContextMenuItem } from './store';

export const [railDetail, setRailDetail] = createSignal<{ session: AgentSessionSummary; anchor: HTMLElement; persistent: boolean } | null>(null);
let closeTimer: ReturnType<typeof setTimeout> | undefined;
let dismissedAnchor: HTMLElement | undefined;
export function keepRailDetail() { clearTimeout(closeTimer); }
export function closeRailDetail() { keepRailDetail(); if (railDetail()?.persistent) dismissedAnchor = railDetail()!.anchor; setRailDetail(null); }
export function leaveRailDetail() {
  keepRailDetail();
  closeTimer = setTimeout(() => {
    const current = railDetail();
    const hovered = current?.anchor.matches(':hover') || document.querySelector('.rail-row-details')?.matches(':hover');
    if (!current?.persistent && !hovered && !current?.anchor.contains(document.activeElement)) setRailDetail(null);
  }, 150);
}

/** Hover and focus show identical metadata. The visible menu entry pins it
 * open for touch, keyboard or a pointer that cannot hold a hover. */
export function railRowSummary(session: () => AgentSessionSummary | undefined) {
  let anchor: HTMLElement | undefined;
  onCleanup(() => { if (railDetail()?.anchor === anchor) closeRailDetail(); });
  const show = (e: Event) => {
    keepRailDetail();
    if (railDetail()?.persistent) return;
    const summary = session();
    if (!summary) { closeRailDetail(); return; }
    anchor = e.currentTarget as HTMLElement;
    if (e.type === 'focusin' && anchor === dismissedAnchor) return;
    dismissedAnchor = undefined;
    setRailDetail({ session: summary, anchor, persistent: false });
  };
  return {
    onPointerEnter: show,
    onPointerLeave: leaveRailDetail,
    onFocusIn: show,
    onFocusOut(e: FocusEvent) {
      if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) leaveRailDetail();
    },
  };
}

export function withRailDetails(items: ContextMenuItem[], session: AgentSessionSummary | undefined): ContextMenuItem[] {
  if (!session) return items;
  return [...items, { label: 'Details', action: () => {
    const anchor = [...document.querySelectorAll<HTMLElement>('[data-session-key]')]
      .find(row => row.dataset.sessionKey === session.key && !!row.closest('.work-rail'));
    if (anchor) { keepRailDetail(); setRailDetail({ session, anchor, persistent: true }); }
  } }];
}
