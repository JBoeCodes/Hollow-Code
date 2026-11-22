import { Show, createEffect, createSignal, onCleanup, onMount } from 'solid-js';
import { createCanvasFocus } from '../canvasFocus';
import { motionReduced as reduced } from '../motion';
import { Icon } from '../icons';
import { appendToChatDraft } from '../store/chat';
import { ChatCanvas } from './ChatCanvas';

/** The Visuals pane's selected canvas. Its heading is the focus target: a
 * tall canvas can still fill the viewport after its beginning has gone away.
 * Kept in a lazy module so the shell doesn't import the canvas renderer. */
export function ExpandedCanvas(props: { code: string; title: string }) {
  let scroller!: HTMLDivElement;
  let content!: HTMLDivElement;
  let target!: HTMLDivElement;
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  let chipFocused = false;
  const [tick, setTick] = createSignal(0);
  const focus = createCanvasFocus({
    scrollTop: () => scroller?.scrollTop ?? 0,
    setScrollTop: (top, smooth) => {
      if (smooth && !reduced()) scroller.scrollTo({ top, behavior: 'smooth' });
      else scroller.scrollTop = top;
    },
    clientHeight: () => scroller?.clientHeight ?? 0,
    scrollHeight: () => scroller?.scrollHeight ?? 0,
    canvasBox: () => {
      if (!scroller || !target) return null;
      const top = target.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
      return { top, bottom: top + 44 };
    },
  }, () => setTick(n => n + 1));
  const chip = () => { tick(); return focus.chip(); };
  createEffect(() => {
    if (!chip() && chipFocused) {
      chipFocused = false;
      queueMicrotask(() => {
        if (target?.isConnected && (!document.activeElement || document.activeElement === document.body)) target.focus({ preventScroll: true });
      });
    }
  });
  const back = () => {
    focus.backToCanvas(true);
    target.focus({ preventScroll: true });
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => focus.settle(), 700);
  };
  onMount(() => {
    focus.turnEnded({ hasCanvas: true, following: true });
    focus.anchor(false);
    const observer = new ResizeObserver(() => focus.resized());
    observer.observe(scroller);
    observer.observe(content);
    onCleanup(() => observer.disconnect());
  });
  onCleanup(() => { clearTimeout(settleTimer); focus.released(); });
  return <div class="side-expanded-canvas">
    <div class="side-visual-body side-canvas-scroll" ref={scroller}
      tabIndex={0} role="region" aria-label={`Scroll canvas: ${props.title}`}
      onScroll={() => focus.scrolled()} onWheel={() => focus.userScrolled()}
      onTouchMove={() => focus.userScrolled()} onPointerDown={e => { if (e.target === scroller) focus.userScrolled(); }}
      onKeyDown={e => {
        if ((e.target === scroller || e.target === target) && ['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' '].includes(e.key)) focus.userScrolled();
      }}>
      <div ref={content}>
        <div ref={target} tabIndex={-1} role="region" aria-label={`Expanded canvas: ${props.title}`}>
          <ChatCanvas code={props.code} closed inPanel compact onAsk={appendToChatDraft} />
        </div>
      </div>
    </div>
    <Show when={chip()}>
      <button type="button" class="side-canvas-back" aria-label={`Back to canvas: ${props.title}`}
        title="Back to canvas (Escape hides this until you reopen the visual)"
        onFocus={() => { chipFocused = true; }} onBlur={() => { if (chip()) chipFocused = false; }}
        onClick={back} onKeyDown={e => {
          if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); focus.dismiss(); target.focus({ preventScroll: true }); }
        }}><Icon name="arrowUp" size={13} />Back to canvas</button>
    </Show>
  </div>;
}
