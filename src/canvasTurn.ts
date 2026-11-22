/**
 * Where a conversation shows a canvas (Canvas C4 A3): "waiting on you"
 * reopens a conversation at its ask-canvas, and the chat looks for the turn
 * here. Its own module so the App chunk doesn't carry it (only the lazy
 * ChatBody does). Pure, so the node Vitest suite covers it.
 */
import { parseSegments } from './chatSegments';
import { CANVAS_END, CANVAS_HEAD, rootAttr, type VisualTurn } from './chatVisuals';

/** A finished canvas's root id (`[a-z0-9-]{1,64}`), by the same light look,
 *  or null. */
export function canvasRootId(code: string): string | null {
  const head = CANVAS_HEAD.exec(code)?.[1];
  if (head === undefined || !CANVAS_END.test(code)) return null;
  const m = rootAttr(head, 'id');
  const id = m ? (m[1] ?? m[2] ?? m[3] ?? '') : '';
  return /^[a-z0-9-]{1,64}$/.test(id) ? id : null;
}

/** The newest assistant turn showing the finished canvas `id`, or -1: where
 *  "waiting on you" reopens a conversation at its ask-canvas (Canvas C4 A3). */
export function canvasTurnIndex(turns: readonly VisualTurn[], id: string): number {
  for (let i = turns.length - 1; i >= 0; i--) {
    const t = turns[i];
    if (t.role !== 'assistant' || !t.text.includes(id)) continue;
    if (parseSegments(t.text).some(seg => seg.kind === 'canvas' && seg.closed && canvasRootId(seg.code) === id)) return i;
  }
  return -1;
}
