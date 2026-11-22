import { createRoot } from 'solid-js';
import { afterEach, expect, it, vi } from 'vitest';
import { draggedRailChat, railChatDragSource, setDraggedRailChat } from './railChatDrag';
import { SESSION_DRAG_MIME } from './sessionDrag';

vi.mock('./store', () => ({ canLeaveConversation: () => true, flashFooter: vi.fn() }));
afterEach(() => { setDraggedRailChat(null); vi.unstubAllGlobals(); });

function harness(key: string) {
  const ghost = { setAttribute: vi.fn(), remove: vi.fn(), className: '', textContent: '' };
  vi.stubGlobal('document', { createElement: () => ghost, body: { append: vi.fn() } });
  const menu = vi.fn(() => [{ label: 'Pin conversation', action: vi.fn() }]);
  const root = createRoot(dispose => ({ source: railChatDragSource(() => key, () => 'Example', menu), dispose }));
  const dataTransfer = { setData: vi.fn(), setDragImage: vi.fn(), effectAllowed: '' };
  const event = {
    target: { closest: () => null }, currentTarget: { classList: { add: vi.fn(), remove: vi.fn() } },
    dataTransfer, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; },
  };
  return { ...root, ghost, dataTransfer, menu, event: event as unknown as DragEvent };
}

it('only the active source can clear its drag when rows dispose', () => {
  const a = harness('s-a'), b = harness('s-b');
  a.source.onDragStart(a.event);
  b.dispose(); expect(draggedRailChat()?.key).toBe('s-a');
  a.dispose(); expect(draggedRailChat()).toBeNull();
});

it('preserves the chat-to-project payload and the exact menu supplier', () => {
  const h = harness('s-target'); h.source.onDragStart(h.event);
  expect(h.dataTransfer.setData).toHaveBeenCalledWith(SESSION_DRAG_MIME, JSON.stringify({ kind: 'session', key: 's-target' }));
  expect(draggedRailChat()?.menu).toBe(h.menu);
  h.source.onDragEnd(h.event); expect(draggedRailChat()).toBeNull();
  expect(h.ghost.remove).toHaveBeenCalledOnce(); h.dispose();
});

it('does not start a chat drag from a rename input', () => {
  const h = harness('s-target');
  (h.event.target as unknown as { closest: () => object | null }).closest = () => ({});
  h.source.onDragStart(h.event);
  expect(h.event.defaultPrevented).toBe(true); expect(h.dataTransfer.setData).not.toHaveBeenCalled();
  expect(draggedRailChat()).toBeNull(); h.dispose();
});
