import { Show } from 'solid-js';
import { Icon } from '../icons';
import { flashFooter, openContextMenuAt, showPrompt, type ContextMenuItem } from '../store';
import { applySavedRailView, deleteSavedRailView, patchRailView, railView, resetRailView, saveCurrentRailView, savedRailViews } from '../store/railView';
import { selecting, startSelecting, stopSelecting } from '../store/rowSelection';
import { FILTER_LABELS, GROUP_LABELS, SORT_LABELS, isDefaultView, sameView, viewSummary, type RailFilter, type RailGroup, type RailSort } from '../railView';

const mark = (on: boolean, label: string) => `${on ? '✓ ' : '   '}${label}`;

/** The Quick chats heading's View menu: sort, filter, group and saved views. */
export function railViewItems(): ContextMenuItem[] {
  const v = railView();
  const items: ContextMenuItem[] = [{ label: 'Sort', disabled: true }];
  for (const id of Object.keys(SORT_LABELS) as RailSort[]) items.push({ label: mark(v.sort === id, SORT_LABELS[id]), action: () => patchRailView({ sort: id }) });
  items.push({ separator: true }, { label: 'Show', disabled: true });
  for (const id of Object.keys(FILTER_LABELS) as RailFilter[]) items.push({ label: mark(v.filter === id, FILTER_LABELS[id]), action: () => patchRailView({ filter: id }) });
  items.push({ separator: true }, { label: 'Group', disabled: true });
  for (const id of Object.keys(GROUP_LABELS) as RailGroup[]) items.push({ label: mark(v.group === id, GROUP_LABELS[id]), action: () => patchRailView({ group: id }) });
  const saved = savedRailViews();
  items.push({ separator: true }, { label: 'Saved views', disabled: true });
  for (const s of saved) items.push({ label: mark(sameView(s, v), s.name), title: viewSummary(s), action: () => applySavedRailView(s.id) });
  items.push({ label: 'Save this view…', disabled: isDefaultView(v) || saved.some(s => sameView(s, v)), title: isDefaultView(v) ? 'Choose a sort, filter or grouping first.' : viewSummary(v),
    action: async () => {
      const name = await showPrompt({ title: 'Save this view', confirmLabel: 'Save', input: { label: 'Name', value: viewSummary(v).slice(0, 40), maxLength: 40 } });
      if (name) { saveCurrentRailView(name); flashFooter(`Saved view “${name}”`); }
    } });
  for (const s of saved) items.push({ label: `Delete saved view “${s.name}”`, danger: true, action: () => { deleteSavedRailView(s.id); flashFooter(`Deleted saved view “${s.name}”`); } });
  if (!isDefaultView(v)) items.push({ separator: true }, { label: 'Reset view', action: () => resetRailView() });
  return items;
}

/** The two icon buttons at the Quick chats heading: Select chats, and View. */
export function RailViewButtons() {
  return <>
    <button type="button" class="icon-btn" aria-pressed={selecting()} aria-label="Select chats" title="Select chats (or Cmd-click a row)" onClick={() => selecting() ? stopSelecting() : startSelecting()}><Icon name="checkSquare" size={15}/></button>
    <button type="button" class="icon-btn rail-view-btn" classList={{ active: !isDefaultView(railView()) }} aria-haspopup="menu" aria-label={`View: ${viewSummary(railView())}`} title={`View: ${viewSummary(railView())}`}
      on:click={e => { e.preventDefault(); e.stopPropagation(); openContextMenuAt(e.currentTarget as Element, railViewItems()); }}><Icon name="list" size={15}/><Show when={!isDefaultView(railView())}><span class="rail-view-dot" aria-hidden="true"/></Show></button>
  </>;
}
