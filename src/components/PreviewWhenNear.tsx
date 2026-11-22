import { createSignal, onCleanup, onMount, type JSX } from 'solid-js';

/** Retain a preview after it reaches its scrolling list's viewport. The
 * caller supplies the listing's read quota; unseen tiles keep a kind icon. */
export function PreviewWhenNear(props: { allowed: boolean; children: (enabled: boolean) => JSX.Element }) {
  let tile!: HTMLDivElement;
  const [seen, setSeen] = createSignal(typeof IntersectionObserver === 'undefined');
  onMount(() => {
    if (seen()) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting)) {
        setSeen(true);
        observer.disconnect();
      }
    }, { root: tile.closest('.cg-list, .library-results-view'), rootMargin: '80px 0px' });
    observer.observe(tile);
    onCleanup(() => observer.disconnect());
  });
  return <div ref={tile}>{props.children(props.allowed && seen())}</div>;
}
