import { Show, createEffect, createSignal, createUniqueId, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import { railDetail, closeRailDetail, keepRailDetail, leaveRailDetail } from '../railRowDetails';
import { sessionState } from '../store/sessionStates';
import { conversationOrganization } from '../store/conversationOrganization';
import { contextMenu, sidebarCollapsed } from '../store';
import { modalOpen } from '../modalGuard';
import '../styles/rail-row-details.css';

/** Metadata already held by the rail, never an inferred conversation recap.
 * The portal avoids the sidebar scroller clipping the card. */
export default function RailRowDetails() {
  let card: HTMLDivElement | undefined;
  const id = `rail-detail-${createUniqueId()}`;
  const [position, setPosition] = createSignal({ left: '0px', top: '0px' });
  const place = () => {
    const current = railDetail();
    if (!current) return;
    if (!current.anchor.isConnected) { closeRailDetail(); return; }
    const anchor = current.anchor.getBoundingClientRect();
    const width = card?.offsetWidth ?? 300, height = card?.offsetHeight ?? 200;
    setPosition({ left: `${Math.max(8, Math.min(anchor.right + 8, window.innerWidth - width - 8))}px`, top: `${Math.max(8, Math.min(anchor.top, window.innerHeight - height - 8))}px` });
  };
  const close = () => {
    const current = railDetail(); closeRailDetail();
    if (current?.persistent && current.anchor.isConnected) current.anchor.querySelector<HTMLElement>('[data-key-item]')?.focus();
  };
  onMount(() => {
    const escape = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || !railDetail()) return;
      // Row rename fields and open menus own Escape before a passive card.
      if (modalOpen() || contextMenu() || (e.target instanceof Element && e.target.closest('input, textarea, [contenteditable="true"]'))) { closeRailDetail(); return; }
      e.preventDefault(); e.stopPropagation(); close();
    };
    const outside = (e: PointerEvent) => { if (railDetail() && !card?.contains(e.target as Node) && !railDetail()!.anchor.contains(e.target as Node)) closeRailDetail(); };
    document.addEventListener('keydown', escape, true);
    document.addEventListener('pointerdown', outside);
    document.addEventListener('dragstart', closeRailDetail, true);
    window.addEventListener('resize', place); document.addEventListener('scroll', place, true);
    onCleanup(() => {
      document.removeEventListener('keydown', escape, true); document.removeEventListener('pointerdown', outside);
      document.removeEventListener('dragstart', closeRailDetail, true);
      window.removeEventListener('resize', place); document.removeEventListener('scroll', place, true); closeRailDetail();
    });
  });
  createEffect(() => {
    const current = railDetail();
    if (!current) return;
    if (sidebarCollapsed() || (contextMenu() && !current.persistent)) { closeRailDetail(); return; }
    const target = current.anchor.querySelector<HTMLElement>('[data-key-item]');
    const previous = target?.getAttribute('aria-describedby');
    target?.setAttribute('aria-describedby', [previous, id].filter(Boolean).join(' '));
    queueMicrotask(() => { if (railDetail() !== current) return; place(); if (current.persistent) card?.focus(); });
    onCleanup(() => { if (previous) target?.setAttribute('aria-describedby', previous); else target?.removeAttribute('aria-describedby'); });
  });
  return <Show when={railDetail()}>{current => <Portal>
    <div ref={card} id={id} class="rail-row-details" style={position()} tabIndex={-1}
      role={current().persistent ? 'dialog' : 'tooltip'} aria-label={current().persistent ? 'Conversation details' : undefined}
      onPointerEnter={keepRailDetail} onPointerLeave={leaveRailDetail} onFocusIn={keepRailDetail}
      onFocusOut={e => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) { if (current().persistent) closeRailDetail(); else leaveRailDetail(); } }}>
      <strong>{current().session.title?.trim() || 'New chat'}</strong>
      <dl>
        <dt>Status</dt><dd>{sessionState(current().session.key)}</dd>
        <dt>Turns</dt><dd>{current().session.turnCount}</dd>
        <dt>Last activity</dt><dd>{current().session.lastActivityAt}</dd>
        <dt>Folder</dt><dd>{current().session.cwd}</dd>
        <Show when={conversationOrganization()[current().session.key]?.reason}>{reason => <><dt>Organization</dt><dd>{reason()}</dd></>}</Show>
        <Show when={current().session.parentKey}>{parent => <><dt>Parent conversation</dt><dd>{parent()}</dd></>}</Show>
      </dl>
      <Show when={current().persistent}><button type="button" class="btn small" onClick={close}>Close details</button></Show>
    </div>
  </Portal>}</Show>;
}
