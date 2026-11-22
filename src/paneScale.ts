import { onCleanup } from 'solid-js';

/**
 * Split panes (the workspace conversation and side panel, Conversation
 * history's transcript and panel) report their width to the resize handle.
 * They no longer scale their text with it: R03 requires that resizing a pane
 * reflows its content at the same type size, so chat and the context pane
 * read at one scale (styles/tokens.css owns the prose tokens).
 */
export function observePaneWidth(el: HTMLElement, onWidth: (width: number) => void) {
  if (typeof ResizeObserver === 'undefined') return;
  const ro = new ResizeObserver(entries => {
    // Border box: a caller resizing the pane sets its outer width.
    const entry = entries[entries.length - 1];
    const w = entry?.borderBoxSize?.[0]?.inlineSize ?? entry?.contentRect.width;
    if (w) onWidth(w);
  });
  ro.observe(el);
  onCleanup(() => ro.disconnect());
}

