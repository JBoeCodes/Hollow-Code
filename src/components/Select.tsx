import { For, Show, createEffect, createMemo, createSignal, createUniqueId, onCleanup, type JSX } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Icon } from '../icons';
import { filterOptions, groupOptions, nextEnabled, typeaheadMatch, type SelectOption } from '../selectModel';
import '../styles/select.css';

export type { SelectOption } from '../selectModel';

/** True when an event target is inside a Select popover. Popovers are
 *  portalled to <body>, so a surrounding popover's "outside press closes me"
 *  handler must ignore them or choosing an option would close its host. */
export const inSelectPopover = (target: EventTarget | null) => !!(target as Element | null)?.closest?.('.hs-pop');

/** Lists at or above this length get a search field (R01: long lists search). */
const SEARCH_AT = 9;

type Common = {
  /** Accessible name. Required: replacing a native select must not lose it. */
  label: string;
  options: SelectOption[];
  placeholder?: string;
  /** `true`/`false` force the search field; default: on for long lists. */
  searchable?: boolean;
  disabled?: boolean;
  class?: string;
  id?: string;
  size?: 'sm' | 'md';
  /** Popover aligns to the trigger's start (default) or end edge. */
  align?: 'start' | 'end';
  /** Shown in the list when a search matches nothing. */
  emptyText?: string;
  /** Replaces the trigger's text (e.g. an icon plus the label). */
  trigger?: (selected: SelectOption[]) => JSX.Element;
  /** A plain trigger with no border (panel headers, toolbars). */
  quiet?: boolean;
  /** Ids of the text that describes the choice (`aria-describedby`). */
  describedBy?: string;
};

/**
 * Hollow's one themed single-choice control (audit R01). A button opens a
 * compact listbox in Hollow's surface/border/accent tokens, portalled above
 * clipping panes. Keyboard: arrows/Home/End/PageUp/PageDown move, Enter or
 * Space chooses, typing jumps (typeahead) or filters (search), Escape and Tab
 * close and return focus to the trigger. Screen readers get combobox/listbox
 * roles with aria-activedescendant, like the native select they replace.
 */
export function Select(props: Common & { value: string; onChange: (value: string, query?: string) => void }) {
  return <Picker {...props} multiple={false} values={() => [props.value]} choose={(v, q) => props.onChange(v, q)} />;
}

/** The same control for several choices, with chips on the trigger. It stays
 *  open while choosing; replaces modifier-click native multiselects (H11). */
export function MultiSelect(props: Common & { value: string[]; onChange: (value: string[]) => void }) {
  return <Picker {...props} multiple values={() => props.value} choose={v => {
    const set = new Set(props.value);
    if (set.has(v)) set.delete(v); else set.add(v);
    // Values without an option (an archived Notion project) are kept, never dropped.
    const known = new Set(props.options.map(o => o.value));
    props.onChange([...props.value.filter(x => !known.has(x) && set.has(x)), ...props.options.map(o => o.value).filter(x => set.has(x))]);
  }} />;
}

function Picker(props: Common & { multiple: boolean; values: () => string[]; choose: (value: string, query?: string) => void }) {
  const uid = createUniqueId();
  const listId = `hs-list-${uid}`;
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [active, setActive] = createSignal(-1);
  const [place, setPlace] = createSignal<{ left: number; top: number; width: number; maxHeight: number; above: boolean }>({ left: 0, top: 0, width: 220, maxHeight: 320, above: false });
  let trigger!: HTMLButtonElement;
  let pop: HTMLDivElement | undefined;
  let search: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  let typed = ''; let typedAt = 0;

  const searchable = () => props.searchable ?? props.options.length >= SEARCH_AT;
  const shown = createMemo(() => filterOptions(props.options, searchable() ? query() : ''));
  const selected = createMemo(() => props.options.filter(o => props.values().includes(o.value)));
  const optionId = (i: number) => `${listId}-${i}`;

  function position() {
    const r = trigger.getBoundingClientRect();
    const vh = window.innerHeight, vw = window.innerWidth;
    const width = Math.min(Math.max(r.width, 200), 380, vw - 16);
    const below = vh - r.bottom - 8, aboveRoom = r.top - 8;
    const above = below < 220 && aboveRoom > below;
    const maxHeight = Math.max(120, Math.min(340, above ? aboveRoom : below));
    let left = props.align === 'end' ? r.right - width : r.left;
    left = Math.min(Math.max(8, left), vw - width - 8);
    setPlace({ left, top: above ? r.top - 4 : r.bottom + 4, width, maxHeight, above });
  }
  function show(start?: number) {
    if (props.disabled || open()) return;
    setQuery('');
    position();
    const first = props.options.findIndex(o => props.values().includes(o.value));
    setActive(start ?? (first >= 0 ? first : nextEnabled(props.options, -1, 1)));
    setOpen(true);
    queueMicrotask(() => { (searchable() ? search : list)?.focus({ preventScroll: true }); reveal(); });
  }
  function hide(refocus = true) {
    if (!open()) return;
    setOpen(false);
    if (refocus) trigger.focus({ preventScroll: true });
  }
  function reveal() { list?.querySelector<HTMLElement>(`#${CSS.escape(optionId(active()))}`)?.scrollIntoView({ block: 'nearest' }); }
  function pick(i: number) {
    const o = shown()[i];
    if (!o || o.disabled) return;
    // The words that found it go along (a Settings section picked by one of its controls lands on it).
    props.choose(o.value, searchable() ? query().trim() || undefined : undefined);
    if (!props.multiple) hide();
  }
  createEffect(() => { if (open()) { shown(); if (active() >= shown().length) setActive(nextEnabled(shown(), -1, 1)); } });

  function move(delta: number, e: KeyboardEvent) {
    e.preventDefault();
    const opts = shown();
    const from = active();
    let next: number;
    if (delta === Infinity) next = nextEnabled(opts, opts.length, -1);
    else if (delta === -Infinity) next = nextEnabled(opts, -1, 1);
    else if (Math.abs(delta) > 1) { next = Math.min(opts.length - 1, Math.max(0, from + delta)); if (opts[next]?.disabled) next = nextEnabled(opts, next, delta > 0 ? 1 : -1); }
    else next = nextEnabled(opts, from, delta);
    if (next >= 0) { setActive(next); queueMicrotask(reveal); }
  }
  // Native (non-delegated) handler so Escape stops here instead of also
  // closing a surrounding modal or side panel that listens on document.
  function popKey(e: KeyboardEvent) {
    // Keys typed in an open popover belong to it. Solid delegates portalled
    // events to the host component, so without this a typeahead "a" or "j"
    // would also reach page shortcuts (accept a suggestion, move a list).
    if (e.key !== 'Tab') e.stopPropagation();
    switch (e.key) {
      case 'ArrowDown': return move(1, e);
      case 'ArrowUp': return move(-1, e);
      case 'PageDown': return move(8, e);
      case 'PageUp': return move(-8, e);
      case 'Home': if (!searchable()) return move(-Infinity, e); return;
      case 'End': if (!searchable()) return move(Infinity, e); return;
      case 'Enter': e.preventDefault(); e.stopPropagation(); return pick(active());
      case ' ': if (!searchable()) { e.preventDefault(); return pick(active()); } return;
      case 'Escape': e.preventDefault(); e.stopPropagation(); return hide();
      // Refocus the trigger; the default Tab then moves on from it.
      case 'Tab': hide(); return;
    }
    if (!searchable() && e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      const now = Date.now();
      typed = now - typedAt > 600 ? e.key : typed + e.key; typedAt = now;
      const i = typeaheadMatch(shown(), typed, active());
      if (i >= 0) { setActive(i); queueMicrotask(reveal); }
    }
  }
  function triggerKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter' || e.key === ' ') { e.preventDefault(); show(); return; }
    // Typing on a closed control opens it at the first match, like native.
    if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey && !props.disabled) {
      e.preventDefault();
      if (searchable()) { show(); setQuery(e.key); setActive(nextEnabled(filterOptions(props.options, e.key), -1, 1)); }
      else { const i = typeaheadMatch(props.options, e.key, -1); show(i >= 0 ? i : undefined); }
    }
  }

  // A control disabled mid-choice (e.g. while saving) closes its list.
  createEffect(() => { if (props.disabled) hide(false); });

  // Dismiss on outside press and keep the popover pinned to its trigger.
  createEffect(() => {
    if (!open()) return;
    const outside = (e: PointerEvent) => { const t = e.target as Node; if (!pop?.contains(t) && !trigger.contains(t)) hide(false); };
    const reflow = () => position();
    document.addEventListener('pointerdown', outside, true);
    window.addEventListener('resize', reflow);
    window.addEventListener('scroll', reflow, true);
    onCleanup(() => { document.removeEventListener('pointerdown', outside, true); window.removeEventListener('resize', reflow); window.removeEventListener('scroll', reflow, true); });
  });

  const text = () => {
    const s = selected();
    if (!s.length) return props.placeholder ?? 'Choose…';
    if (props.multiple) return s.length <= 2 ? s.map(o => o.label).join(', ') : `${s[0].label} +${s.length - 1}`;
    return s[0].label;
  };

  return <>
    <button ref={trigger} type="button" id={props.id} class={`hs-select ${props.class ?? ''}`} classList={{ sm: props.size === 'sm', quiet: !!props.quiet, open: open(), placeholder: !selected().length }}
      role="combobox" aria-haspopup="listbox" aria-expanded={open()} aria-controls={open() ? listId : undefined} aria-label={props.label} aria-describedby={props.describedBy}
      disabled={props.disabled} title={props.multiple ? text() : undefined} data-value={props.multiple ? props.values().join(',') : props.values()[0] ?? ''}
      onClick={() => open() ? hide() : show()} on:keydown={triggerKey}>
      <span class="hs-select-value">{props.trigger ? props.trigger(selected()) : text()}</span>
      <Icon name="chevronDown" size={13} />
    </button>
    <Show when={open()}>
      <Portal>
        <div ref={pop} class="hs-pop" classList={{ above: place().above }} on:keydown={popKey}
          style={{ left: `${place().left}px`, top: `${place().top}px`, width: `${place().width}px`, 'max-height': `${place().maxHeight}px` }}>
          <Show when={searchable()}>
            <div class="hs-search"><Icon name="search" size={13} />
              <input ref={search} type="text" role="combobox" aria-label={`Search ${props.label}`} aria-expanded="true" aria-controls={listId} aria-autocomplete="list"
                aria-activedescendant={active() >= 0 ? optionId(active()) : undefined} placeholder="Search…" value={query()}
                onInput={e => { setQuery(e.currentTarget.value); setActive(nextEnabled(filterOptions(props.options, e.currentTarget.value), -1, 1)); }} />
            </div>
          </Show>
          <div ref={list} id={listId} class="hs-list" role="listbox" aria-label={props.label} aria-multiselectable={props.multiple || undefined} tabindex={searchable() ? undefined : -1}
            aria-activedescendant={!searchable() && active() >= 0 ? optionId(active()) : undefined}>
            <Show when={shown().length} fallback={<div class="hs-empty">{props.emptyText ?? 'No matches'}</div>}>
              <For each={groupOptions(shown())}>{g => <>
                <Show when={g.group}><div class="hs-group" role="presentation">{g.group}</div></Show>
                <For each={g.items}>{({ option, index }) => {
                  const on = () => props.values().includes(option.value);
                  return <div id={optionId(index)} role="option" class="hs-option" data-value={option.value} aria-selected={on()} aria-disabled={option.disabled || undefined}
                    classList={{ active: active() === index, selected: on(), disabled: !!option.disabled }}
                    onPointerMove={() => { if (!option.disabled && active() !== index) setActive(index); }}
                    onPointerDown={e => e.preventDefault()} onClick={() => pick(index)}>
                    <span class="hs-check" aria-hidden="true"><Show when={on()}><Icon name="check" size={13} /></Show></span>
                    <span class="hs-option-text"><span class="hs-option-label">{option.label}</span><Show when={option.detail}><small>{option.detail}</small></Show></span>
                  </div>;
                }}</For>
              </>}</For>
            </Show>
          </div>
        </div>
      </Portal>
    </Show>
  </>;
}
