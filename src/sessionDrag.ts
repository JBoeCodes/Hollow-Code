/**
 * The conversation drag contract (2026-09-30): what a dragged conversation
 * row carries and how a drop onto a workspace header is dispatched. Pure (no
 * store, no DOM), so vitest covers it; `dragdrop.ts` re-exports it and adds
 * the JSX handlers the rail and Conversation history use.
 *
 * - The payload rides in `DataTransfer` under {@link SESSION_DRAG_MIME} as
 *   JSON: `{ kind: 'session', key }` for a Hollow conversation. Another kind
 *   would use the same MIME type and register its own handler; drop targets
 *   never change for a new kind. (The session importers' `external-session`
 *   kind went with From other apps in run 15 P4.)
 * - A drop target calls {@link dispatchSessionDrop} with the raw data and its
 *   workspace id; the handler registered for the payload's kind does the work.
 */

export const SESSION_DRAG_MIME = 'application/x-hollow-session';

/** A Hollow conversation being dragged. */
export type SessionDragPayload = { kind: 'session'; key: string };

/** Any payload under the MIME type: a `kind` plus that kind's fields. */
export type AnySessionDragPayload = { kind: string } & Record<string, unknown>;

export type SessionDropHandler<P extends AnySessionDragPayload = AnySessionDragPayload> =
  (payload: P, workspaceId: string) => Promise<void>;

const handlers = new Map<string, SessionDropHandler>();

/** Register what a drop of payload `kind` does. Returns the unregister
 *  function; a second registration for the same kind replaces the first. */
export function registerSessionDropHandler<P extends AnySessionDragPayload>(kind: P['kind'], handler: SessionDropHandler<P>): () => void {
  const entry = handler as SessionDropHandler;
  handlers.set(kind, entry);
  return () => { if (handlers.get(kind) === entry) handlers.delete(kind); };
}

/** Hollow session keys (`agent_sessions::is_safe_session_key`). */
const SESSION_KEY = /^s-[a-z0-9-]{1,120}$/;

/** The JSON a drag carries. */
export function encodeSessionDrag(payload: AnySessionDragPayload): string {
  return JSON.stringify(payload);
}

/** The payload in a drag's data, or null for anything malformed: not JSON,
 *  not an object, no string `kind`, or a `session` without a valid key. */
export function parseSessionDrag(text: string | null | undefined): AnySessionDragPayload | null {
  if (!text) return null;
  let value: unknown;
  try { value = JSON.parse(text); } catch { return null; }
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const payload = value as AnySessionDragPayload;
  if (typeof payload.kind !== 'string' || !payload.kind) return null;
  if (payload.kind === 'session' && (typeof payload.key !== 'string' || !SESSION_KEY.test(payload.key))) return null;
  return payload;
}

/** Whether a drag (by its `DataTransfer.types`, all a dragover can read)
 *  carries a conversation. */
export function isSessionDrag(types: readonly string[] | DOMStringList | null | undefined): boolean {
  if (!types) return false;
  return Array.from(types as ArrayLike<string>).includes(SESSION_DRAG_MIME);
}

/** Run the registered handler for a drop onto `workspaceId`. False when the
 *  data is malformed or no handler knows its kind (nothing happened). */
export async function dispatchSessionDrop(text: string | null | undefined, workspaceId: string): Promise<boolean> {
  const payload = parseSessionDrag(text);
  if (!payload || !workspaceId) return false;
  const handler = handlers.get(payload.kind);
  if (!handler) return false;
  await handler(payload, workspaceId);
  return true;
}
