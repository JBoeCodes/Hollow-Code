import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { flashFooter } from '../store';
import { draggedRailChat as dragged, setDraggedRailChat as setDragged } from '../railChatDrag';
import { SESSION_DRAG_MIME, parseSessionDrag } from '../sessionDrag';
import { railDropPlan, type RailDropSection } from '../railDrag';
import { conversationOrganization } from '../store/conversationOrganization';
import { conversationBusy } from '../workspaceMenu';
import { observeRailListMotion } from '../railListMotion';
import { Icon } from '../icons';
import '../styles/rail-interactions.css';

/** Empty shelves are still reachable while dragging. Labels name the same
 * commands as each row's visible actions menu; no new persisted operation. */
export default function RailDropZones() {
  let anchor!: HTMLSpanElement;
  onMount(() => {
    const root = anchor.closest<HTMLElement>('.work-rail-scroll');
    if (root) onCleanup(observeRailListMotion(root));
  });
  const [hovered, setHovered] = createSignal<RailDropSection | null>(null);
  const sections = ['pinned', 'active', 'settled'] as const;
  const names = { pinned: 'Pinned', active: 'Active', settled: 'Settled' };
  const plan = (section: RailDropSection) => {
    const key = dragged()?.key;
    return railDropPlan(section, key ? conversationOrganization()[key] : undefined, !!key && !!conversationBusy(key));
  };
  const over = (e: DragEvent, section: RailDropSection) => {
    if (!dragged() || !e.dataTransfer) return;
    e.preventDefault(); e.stopPropagation();
    e.dataTransfer.dropEffect = plan(section).action ? 'move' : 'none';
    setHovered(section);
  };
  const drop = (e: DragEvent, section: RailDropSection) => {
    e.preventDefault(); e.stopPropagation(); setHovered(null);
    const payload = parseSessionDrag(e.dataTransfer?.getData(SESSION_DRAG_MIME));
    const source = dragged(), key = source?.key;
    const selected = plan(section);
    setDragged(null);
    if (!key || payload?.kind !== 'session' || payload.key !== key) return;
    if (!selected.action) { flashFooter(selected.reason ?? selected.label); return; }
    // Invoke the exact current menu entry, including its fresh-chat and focus
    // hooks. Its organization handler owns revision/activity checks and Undo.
    const item = source?.menu().find(item => item.label === selected.label);
    if (!item || item.disabled || !item.action) { flashFooter('This action changed. Reopen the conversation menu.'); return; }
    item.action();
  };
  return <><span ref={anchor} hidden aria-hidden="true"/><Show when={dragged()}>
    <div class="rail-drop-zones" role="group" aria-label="Move conversation between sections">
      <For each={sections}>{section => <div class="rail-drop-zone" classList={{ 'rail-drop-active': hovered() === section, 'rail-drop-refused': !plan(section).action }}
        onDragEnter={e => over(e, section)} onDragOver={e => over(e, section)} onDrop={e => drop(e, section)}
        onDragLeave={e => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHovered(null); }}>
        <Icon name={section === 'pinned' ? 'pin' : section === 'settled' ? 'archive' : 'message'} size={14}/>
        <span>{names[section]}</span><span class="rail-drop-action">{plan(section).label}</span>
      </div>}</For>
    </div>
  </Show></>;
}
