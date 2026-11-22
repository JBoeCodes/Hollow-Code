import { createSignal, onCleanup } from 'solid-js';
import { canLeaveConversation, type ContextMenuItem } from './store';
import { sessionDragSource } from './sessionDragHandlers';
import './styles/rail-interactions.css';

interface RailDrag { key: string; title: string; menu: () => ContextMenuItem[] }
export const [draggedRailChat, setDraggedRailChat] = createSignal<RailDrag | null>(null);

/** Keep this source helper small: nested rows live in the launch chunk.
 * The destinations and list motion load separately through RailDropZones. */
export function railChatDragSource(key: () => string, title: () => string, menu: () => ContextMenuItem[]) {
  const source = sessionDragSource(() => ({ kind: 'session', key: key() }), () => canLeaveConversation() ? null : 'Wait for the current message to finish sending.');
  let ghost: HTMLElement | undefined;
  let owned: RailDrag | undefined;
  const clear = () => { ghost?.remove(); ghost = undefined; if (draggedRailChat() === owned) setDraggedRailChat(null); owned = undefined; };
  onCleanup(clear);
  return {
    draggable: true,
    onDragStart(e: DragEvent) {
      if ((e.target as Element).closest('input, textarea, .work-row-actions')) { e.preventDefault(); return; }
      source.onDragStart(e);
      if (e.defaultPrevented || !e.dataTransfer) return;
      clear();
      const identity = { key: key(), title: title(), menu };
      owned = identity;
      setDraggedRailChat(identity);
      ghost = document.createElement('div');
      ghost.className = 'rail-drag-ghost';
      ghost.setAttribute('aria-hidden', 'true');
      ghost.textContent = identity.title;
      document.body.append(ghost);
      e.dataTransfer.setDragImage?.(ghost, 16, 16);
    },
    onDragEnd(e: DragEvent) { source.onDragEnd(e); clear(); },
  };
}
