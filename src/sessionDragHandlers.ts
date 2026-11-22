// ── Conversations onto workspaces (2026-09-30) ─────────────────────────
// The payload contract and handler registry live in sessionDrag.ts (pure);
// these are the JSX handlers for a draggable conversation row and a
// workspace header that accepts one. Their own module (overnight 12 A0) so the
// rail, which is in the App chunk, doesn't pull dragdrop.ts's task drop
// (`applyDrop`, used only by lazy pages) in with them; dragdrop.ts re-exports
// them for everyone else.
import { flashFooter } from './store';
import { SESSION_DRAG_MIME, dispatchSessionDrop, encodeSessionDrag, isSessionDrag, type AnySessionDragPayload } from './sessionDrag';

/** The class a drop target wears while a conversation hovers over it. */
export const SESSION_DROP_CLASS = 'session-drop-target';

/** Handlers for a draggable conversation row. `blocked` returns why it
 *  can't move right now (a turn in flight, a send still being accepted): the
 *  drag is refused and the reason shown. */
export function sessionDragSource(payload: () => AnySessionDragPayload | null, blocked?: () => string | null | undefined) {
  return {
    draggable: true,
    onDragStart(e: DragEvent) {
      const p = payload();
      const why = blocked?.();
      if (!p || why || !e.dataTransfer) {
        e.preventDefault();
        if (why) flashFooter(`× ${why}`, 4000);
        return;
      }
      e.dataTransfer.setData(SESSION_DRAG_MIME, encodeSessionDrag(p));
      e.dataTransfer.effectAllowed = 'move';
      (e.currentTarget as HTMLElement).classList.add('session-dragging');
    },
    onDragEnd(e: DragEvent) {
      (e.currentTarget as HTMLElement).classList.remove('session-dragging');
    },
  };
}

/** Handlers for a workspace header that accepts dropped conversations. */
export function sessionDropTarget(workspaceId: () => string | null | undefined) {
  const accepts = (e: DragEvent) => !!workspaceId() && isSessionDrag(e.dataTransfer?.types);
  return {
    onDragEnter(e: DragEvent) {
      if (!accepts(e)) return;
      e.preventDefault();
      (e.currentTarget as HTMLElement).classList.add(SESSION_DROP_CLASS);
    },
    onDragOver(e: DragEvent) {
      if (!accepts(e)) return;
      e.preventDefault();
      e.dataTransfer!.dropEffect = 'move';
      (e.currentTarget as HTMLElement).classList.add(SESSION_DROP_CLASS);
    },
    onDragLeave(e: DragEvent) {
      const target = e.currentTarget as HTMLElement;
      if (!target.contains(e.relatedTarget as Node)) target.classList.remove(SESSION_DROP_CLASS);
    },
    onDrop(e: DragEvent) {
      const target = e.currentTarget as HTMLElement;
      target.classList.remove(SESSION_DROP_CLASS);
      const id = workspaceId();
      if (!id || !accepts(e)) return;
      e.preventDefault();
      const data = e.dataTransfer!.getData(SESSION_DRAG_MIME);
      dispatchSessionDrop(data, id).catch(err => flashFooter(`× ${err instanceof Error ? err.message : String(err)}`, 5000));
    },
  };
}
