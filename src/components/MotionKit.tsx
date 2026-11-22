import { For, Show, onCleanup, onMount, type JSX } from 'solid-js';
import { measureMotionSize, observeKitMotion } from '../motionKit';

/** Wrap naturally sized content, including native details. The child owns
 * disclosure semantics and focus; the wrapper only interpolates geometry. */
export function AnimatedHeight(props: { children: JSX.Element; class?: string }) {
  let outer!: HTMLDivElement, inner!: HTMLDivElement;
  onMount(() => onCleanup(measureMotionSize(outer, inner, 'height')));
  return <div ref={outer} class={`motion-height ${props.class ?? ''}`}><div ref={inner} class="motion-height-content">{props.children}</div></div>;
}

export function AnimatedWidth(props: { children: JSX.Element; class?: string }) {
  let outer!: HTMLSpanElement, inner!: HTMLSpanElement;
  onMount(() => onCleanup(measureMotionSize(outer, inner, 'width')));
  return <span ref={outer} class={`motion-width ${props.class ?? ''}`}><span ref={inner} class="motion-width-content">{props.children}</span></span>;
}

/** Original text remains opaque; the moving band is decorative only. */
export function MotionCue(props: { kind: 'tool' | 'activity' | 'search' | 'drag'; active?: boolean; children: JSX.Element; class?: string }) {
  let element!: HTMLSpanElement;
  onMount(() => onCleanup(observeKitMotion(element, () => {})));
  return <span ref={element} class={`motion-cue motion-${props.kind} ${props.class ?? ''}`} data-active={props.active ? 'true' : 'false'}>{props.children}</span>;
}

/** No percentage is announced: the bar describes activity, not measured work.
 * Keep mounted across loading changes to show completion, or unmount directly. */
export function LoadingProgress(props: { loading: boolean; label: string }) {
  let element!: HTMLDivElement;
  onMount(() => onCleanup(observeKitMotion(element, () => {})));
  return <div ref={element} class="motion-progress" data-loading={props.loading ? 'true' : 'false'} role={props.loading ? 'status' : undefined} aria-label={props.loading ? props.label : undefined} aria-busy={props.loading}>
    <span aria-hidden="true" />
  </div>;
}

/** Operation-labelled rows with final-row geometry and decorative bars. */
export function SkeletonRows(props: { rows?: number; label: string; detail?: boolean; class?: string }) {
  let element!: HTMLDivElement;
  onMount(() => onCleanup(observeKitMotion(element, () => {})));
  return <div ref={element} class={`motion-skeleton ${props.class ?? ''}`} data-detail={props.detail ? 'true' : undefined} role="status" aria-busy="true" aria-label={props.label}>
    <For each={Array.from({ length: props.rows ?? 3 }, (_, i) => i)}>{i => <div class="motion-skeleton-row" aria-hidden="true"><span class="motion-skeleton-mark" /><span class="motion-skeleton-text"><span class="motion-skeleton-line" style={{ width: `${[72, 94, 58, 86, 64, 90][i % 6]}%` }} /><Show when={props.detail}><span class="motion-skeleton-detail" /></Show></span></div>}</For>
  </div>;
}
