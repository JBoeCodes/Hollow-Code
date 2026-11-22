// The Tasks page's view tabs, filter bar and Display choices (overhaul B2,
// board S3b), laid out the way a tracker shows them: the views in a row,
// then one chip per filter with its values and a remove button, "Filter"
// to add one, and Group and Order at the end. The model is
// `trackerViews.ts`; the page owns the state and persists it.
import { For, Show } from 'solid-js';
import type { TrackerTeamMeta } from '../../generated/ipc';
import { ActionMenu } from '../../components/ActionMenu';
import { MultiSelect, Select } from '../../components/Select';
import { Icon } from '../../icons';
import {
  FILTER_FIELDS, GROUPINGS, ORDERINGS, TAB_VIEWS, chooseView, emptyFilters, filterOptions, hasFilters, showConflicts,
  type FilterField, type Grouping, type Ordering, type ViewState,
} from '../../trackerViews';

/** The view tabs, then "Conflicts (n)" while any issue has a conflict (the
 *  page's conflicts panel, design section 7): it lists those issues, and
 *  each one's detail offers Use mine (re-apply) or Keep theirs. */
export function ViewTabs(props: { state: ViewState; conflicts: number; onChange: (next: ViewState) => void }) {
  return (
    <div class="tracker-views" role="group" aria-label="Views">
      <For each={TAB_VIEWS}>{v => (
        <button type="button" class="tracker-view" data-view={v.id} aria-pressed={props.state.view === v.id} title={v.hint}
          onClick={() => props.onChange(chooseView(props.state, v.id))}>{v.label}</button>
      )}</For>
      <Show when={showConflicts(props.state, props.conflicts)}>
        <button type="button" class="tracker-view" data-view="conflicts" aria-pressed={props.state.view === 'conflicts'}
          title="Issues where the tracker changed a field you edited, so your edit was not applied. Open one to re-apply it or keep the tracker’s."
          onClick={() => props.onChange(chooseView(props.state, 'conflicts'))}>Conflicts ({props.conflicts})</button>
      </Show>
    </div>
  );
}

/** Fields shown as chips: the ones with values, plus any the person just
 *  added from Filter and hasn't chosen values for yet. */
export function FilterBar(props: {
  state: ViewState;
  meta: TrackerTeamMeta | undefined;
  shown: readonly FilterField[];
  onShow: (field: FilterField) => void;
  onHide: (field: FilterField) => void;
  onChange: (next: ViewState) => void;
}) {
  const label = (f: FilterField) => FILTER_FIELDS.find(x => x.field === f)!.label;
  const setField = (field: FilterField, values: string[]) =>
    props.onChange({ ...props.state, filters: { ...props.state.filters, [field]: values } });
  const addable = () => FILTER_FIELDS.filter(f => !props.shown.includes(f.field));
  return (
    <div class="tracker-filters" role="group" aria-label="Filters">
      <For each={props.shown}>{field => (
        <span class="tracker-filter-chip" data-field={field}>
          <span class="tracker-filter-name">{label(field)}</span>
          <MultiSelect size="sm" quiet label={`${label(field)} filter`} placeholder="any" value={props.state.filters[field]}
            options={filterOptions(field, props.meta)} onChange={v => setField(field, v)} />
          <button type="button" class="tracker-filter-remove" aria-label={`Remove the ${label(field)} filter`} title="Remove this filter"
            onClick={() => { setField(field, []); props.onHide(field); }}><Icon name="x" size={12} /></button>
        </span>
      )}</For>
      <Show when={addable().length}>
        <ActionMenu label="Add a filter" align="start" icon={<span class="tracker-filter-add"><Icon name="plus" size={14} /> Filter</span>}
          items={addable().map(f => ({ label: f.label, action: () => props.onShow(f.field) }))} />
      </Show>
      <Show when={hasFilters(props.state.filters) || props.shown.length}>
        <button type="button" class="btn small ghost tracker-filter-clear"
          onClick={() => { props.onChange({ ...props.state, filters: emptyFilters() }); for (const f of props.shown) props.onHide(f); }}>Clear filters</button>
      </Show>
    </div>
  );
}

export function DisplayChoices(props: { state: ViewState; onChange: (next: ViewState) => void }) {
  return (
    <div class="tracker-display" role="group" aria-label="Display">
      <Select size="sm" quiet label="Group by" value={props.state.grouping} searchable={false}
        options={GROUPINGS.map(g => ({ ...g }))} trigger={sel => <>Group: {sel[0]?.label}</>}
        onChange={v => props.onChange({ ...props.state, grouping: v as Grouping })} />
      <Select size="sm" quiet label="Order by" value={props.state.ordering} searchable={false}
        options={ORDERINGS.map(o => ({ ...o }))} trigger={sel => <>Order: {sel[0]?.label}</>}
        onChange={v => props.onChange({ ...props.state, ordering: v as Ordering })} />
    </div>
  );
}
