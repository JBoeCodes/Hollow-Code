// Which conversation a chat event belongs to (build programme PR B4). Pure,
// so the node vitest suite can load it; the store in store/chatRuntime.ts applies it.

/** The routing fields every tagged chat event carries. */
export interface RoutedEvent {
  type: string;
  clientTurnId?: string | null;
  queueId?: number | null;
  conversationKey?: string | null;
  text?: string;
}

export interface RouteContext {
  /** A conversation held in the background has this key. */
  isBackground: (key: string) => boolean;
  /** The conversation on screen's key (null for a fresh, unsent one). */
  foregroundKey: string | null;
  /** The on-screen turn still being queued, if any. */
  queued: { clientTurnId?: string | null; queueId?: number | null } | null;
}

export type Route = 'foreground' | 'background' | 'orphan';

/** Where an event goes. An untagged event (an older backend) and a fresh
 *  conversation's own first turn stay on screen; another conversation this
 *  window holds is buffered; any other conversation's event is an orphan
 *  (never drawn into the one on screen). */
export function routeEvent(p: RoutedEvent, cx: RouteContext): Route {
  const route = p.conversationKey;
  if (!route) return 'foreground';
  if (cx.isBackground(route)) return 'background';
  if (cx.foregroundKey === route) return 'foreground';
  const q = cx.queued;
  if (q && ((!!p.clientTurnId && q.clientTurnId === p.clientTurnId) || (p.queueId != null && q.queueId === p.queueId))) {
    return 'foreground';
  }
  return 'orphan';
}

/** Append `p` to a background conversation's buffer, joining consecutive
 *  streamed deltas so a long answer stays one entry. */
export function bufferEvent<T extends RoutedEvent>(events: T[], p: T): void {
  const last = events[events.length - 1];
  if (p.type === 'assistantDelta' && last?.type === 'assistantDelta') {
    events[events.length - 1] = { ...last, text: (last.text ?? '') + (p.text ?? '') };
  } else events.push(p);
}
