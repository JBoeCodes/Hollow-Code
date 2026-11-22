import { onCleanup } from 'solid-js';

/**
 * Vertical drag handle for column-resizing. Caller controls clamping in
 * `onResize`. `edge` selects which side of the parent the handle sits on
 * AND which drag direction grows the panel:
 *   - 'right' (default): handle on right edge, dragging right grows it
 *     (used by left-anchored asides like Chat sessions / Vault sidebar).
 *   - 'left': handle on left edge, dragging LEFT grows it (used by the
 *     right-anchored ChatPanel).
 */
export function ResizeHandle(props: {
  /** Current width — used as the starting size at mousedown. */
  current: () => number;
  /** Called continuously during drag with the new (already-clamped) width. */
  onResize: (next: number) => void;
  /** Called once on mouseup so the caller can persist the final value. */
  onCommit?: () => void;
  min: number;
  max: number;
  edge?: 'left' | 'right';
  /** Accessible name. When set the handle is a focusable separator that
   *  arrow keys resize (16px a press, 64px with Shift). */
  label?: string;
  /** Double-click: return to the caller's default size. */
  onReset?: () => void;
}) {
  let startX = 0;
  let startW = 0;
  let active = false;

  function onMouseDown(e: MouseEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    active = true;
    startX = e.clientX;
    startW = props.current();
    document.body.style.cursor = 'col-resize';
    // The side panel's browser hides its native page while a drag is on,
    // or the page would swallow the pointer (BrowserPane.tsx).
    document.body.dataset.resizing = '1';
    document.body.style.userSelect = 'none';
    document.addEventListener('mousemove', onMouseMove);
    document.addEventListener('mouseup', onMouseUp);
  }

  // Coalesce to one width write per frame. mousemove fires far faster than
  // the display refreshes, and every surplus event costs a Solid signal
  // propagation plus an inline style write that the next frame overwrites
  // anyway. Same store-the-event/schedule-one-rAF shape as the torchlight
  // pointermove in App.tsx. Props are read inside the callback, not
  // snapshotted at setup, so a reactive `edge`/`min`/`max` still applies.
  let moveRaf = 0;
  let moveEvt: MouseEvent | null = null;

  const applyMove = () => {
    moveRaf = 0;
    const e = moveEvt;
    if (!e || !active) return;
    const delta = e.clientX - startX;
    const next = props.edge === 'left' ? startW - delta : startW + delta;
    props.onResize(Math.max(props.min, Math.min(props.max, next)));
  };

  /** Apply a pending frame now — used on mouseup so the final pixels of the
   *  drag are not lost to a coalesced event, which would make the committed
   *  width differ from where the user let go. */
  function flushMove() {
    if (!moveRaf) return;
    cancelAnimationFrame(moveRaf);
    applyMove();
  }

  function cancelMove() {
    if (moveRaf) cancelAnimationFrame(moveRaf);
    moveRaf = 0;
    moveEvt = null;
  }

  function onMouseMove(e: MouseEvent) {
    if (!active) return;
    moveEvt = e;
    if (!moveRaf) moveRaf = requestAnimationFrame(applyMove);
  }

  function onMouseUp() {
    if (!active) return;
    flushMove();
    cancelMove();
    active = false;
    document.body.style.cursor = '';
    delete document.body.dataset.resizing;
    document.body.style.userSelect = '';
    document.removeEventListener('mousemove', onMouseMove);
    document.removeEventListener('mouseup', onMouseUp);
    props.onCommit?.();
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const step = (e.shiftKey ? 64 : 16) * (e.key === 'ArrowLeft' ? -1 : 1);
    const next = props.current() + (props.edge === 'left' ? -step : step);
    props.onResize(Math.max(props.min, Math.min(props.max, next)));
    props.onCommit?.();
  }

  onCleanup(() => {
    // Unmounting mid-drag: drop the queued frame rather than flushing it,
    // so a torn-down pane cannot write a width after it is gone.
    cancelMove();
    document.removeEventListener('mousemove', onMouseMove);
    document.removeEventListener('mouseup', onMouseUp);
  });

  return (
    <div
      class={`resize-handle ${props.edge === 'left' ? 'edge-left' : ''}`}
      onMouseDown={onMouseDown}
      onDblClick={() => props.onReset?.()}
      role={props.label ? 'separator' : undefined}
      aria-label={props.label}
      aria-orientation={props.label ? 'vertical' : undefined}
      aria-valuenow={props.label ? Math.round(props.current()) : undefined}
      aria-valuemin={props.label ? props.min : undefined}
      aria-valuemax={props.label ? Math.round(props.max) : undefined}
      tabIndex={props.label ? 0 : undefined}
      onKeyDown={props.label ? onKeyDown : undefined}
      title={props.label ? `${props.label}${props.onReset ? ' · double-click to reset' : ''}` : undefined}
    />
  );
}
