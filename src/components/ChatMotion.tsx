import { onCleanup, onMount } from 'solid-js';
import '../styles/chat-motion.css';

// One observer/listener set for the mounted chat decorations, released with the last root.
const roots = new Map<HTMLElement, boolean>();
let intersection: IntersectionObserver | undefined;
let away: MutationObserver | undefined;
function syncMotion() {
  const visible = !document.hidden && !document.body.classList.contains('is-away');
  for (const [root, inView] of roots) {
    const next = String(visible && inView);
    if (root.dataset.motionVisible !== next) root.dataset.motionVisible = next;
  }
}
/** Decoration only. No frame loop; no retained roots after unmount. */
export function observeChatMotion(root: HTMLElement) {
  onMount(() => {
    if (!roots.size) {
      if (typeof IntersectionObserver !== 'undefined') intersection = new IntersectionObserver(entries => {
        for (const e of entries) if (roots.has(e.target as HTMLElement)) roots.set(e.target as HTMLElement, e.isIntersecting && e.intersectionRatio > 0);
        syncMotion();
      });
      away = new MutationObserver(syncMotion);
      away.observe(document.body, { attributes: true, attributeFilter: ['class'] });
      document.addEventListener('visibilitychange', syncMotion);
    }
    roots.set(root, false); intersection?.observe(root); syncMotion();
    onCleanup(() => {
      intersection?.unobserve(root); roots.delete(root);
      if (!roots.size) {
        intersection?.disconnect(); intersection = undefined; away?.disconnect(); away = undefined;
        document.removeEventListener('visibilitychange', syncMotion);
      }
    });
  });
}

/** The adjacent activity words supply meaning, including Queued and quiet states. */
export function WorkingMark(props: { queued?: boolean }) {
  return <span class="chat-working-mark" data-queued={props.queued ? 'true' : 'false'} data-motion-visible="false" ref={observeChatMotion} aria-hidden="true">
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-linecap="round" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke-width="1" opacity=".3"/>
      <g class="chat-working-orbit"><path d="M12 3a9 9 0 0 1 7.8 13.5" stroke-width="1.8"/><circle cx="19.8" cy="16.5" r="1.3" fill="currentColor" stroke="none"/></g>
      <path d="M9 16v-6a3 3 0 0 1 6 0v6" stroke-width="1.5"/>
    </svg>
  </span>;
}

export function CanvasPreparation(props: { title: string; height: number; loading?: boolean }) {
  return <div class="chat-canvas-skeleton chat-preparation" data-motion-visible="false" ref={observeChatMotion}
    role={props.loading ? undefined : 'status'} style={{ height: `${props.height}px` }}>
    <span class="chat-preparation-lines" aria-hidden="true"><i/><i/><i/></span>
    <span class="chat-canvas-skeleton-label">{props.loading ? 'Loading' : 'Drawing'} <strong>{props.title}</strong>…</span>
    <span class="chat-preparation-sheen" aria-hidden="true"/>
  </div>;
}
