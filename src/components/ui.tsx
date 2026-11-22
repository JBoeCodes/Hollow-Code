import { For, Show, children, createMemo, createSignal, createUniqueId, splitProps, type JSX } from 'solid-js';
import { Icon, type IconName } from '../icons';
import { EmptyArt } from './EmptyArt';
import { emptyStateScene, type EmptyEvidence, type EmptyScene } from '../emptyScene';
import { shownError } from '../plainErrors';
import { errMsg } from '../utils';
export { PageHeader } from './PageHeader';
export { ActionMenu as Menu, type ActionMenuItem as MenuItem } from './ActionMenu';

/**
 * Shared UI primitives (frontier F3). Pages build their buttons, tabs, cards,
 * rows and empty states from these so every surface looks and behaves the
 * same; the styles are in `styles/ui.css`. They render the long-standing class
 * names (`btn`, `page-header`, …) so existing selectors and tests keep working.
 */

type ButtonVariant = 'secondary' | 'primary' | 'ghost' | 'danger';

export function Button(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: 'sm' | 'md';
  icon?: IconName;
}) {
  const [local, rest] = splitProps(props, ['variant', 'size', 'icon', 'class', 'children', 'type']);
  return (
    <button
      type={local.type ?? 'button'}
      class={`btn${local.variant && local.variant !== 'secondary' ? ` ${local.variant}` : ''}${local.size === 'sm' ? ' small' : ''}${local.class ? ` ${local.class}` : ''}`}
      {...rest}
    >
      <Show when={local.icon}><Icon name={local.icon!} size={14} /></Show>
      {local.children}
    </button>
  );
}

/** A square icon-only button. `label` is required: it is the accessible name
 * and the tooltip. `pressed` makes it a toggle. */
export function IconButton(props: JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  icon: IconName;
  label: string;
  pressed?: boolean;
  size?: number;
}) {
  const [local, rest] = splitProps(props, ['icon', 'label', 'pressed', 'size', 'class', 'type']);
  return (
    <button
      type={local.type ?? 'button'}
      class={`icon-btn${local.class ? ` ${local.class}` : ''}`}
      aria-label={local.label}
      title={local.label}
      aria-pressed={local.pressed === undefined ? undefined : local.pressed}
      {...rest}
    >
      <Icon name={local.icon} size={local.size ?? 16} />
    </button>
  );
}

export type TabItem<T extends string> = { value: T; label: string; count?: number | string; hint?: string; accessibleName?: string; icon?: IconName };

/** A row of view tabs (underlined when selected). A view switch within one
 * page, so the buttons are toggles (`aria-pressed`) in a labelled group. */
export function Tabs<T extends string>(props: {
  label: string;
  items: readonly TabItem<T>[];
  value: T;
  onChange: (value: T) => void;
  class?: string;
  level?: 'primary' | 'secondary';
}) {
  const descriptionId = createUniqueId();
  return (
    <nav class={`ui-tabs ui-level-${props.level ?? 'primary'}${props.class ? ` ${props.class}` : ''}`} aria-label={props.label}>
      <For each={props.items}>{item =>
        <button type="button" classList={{ active: props.value === item.value }} aria-label={item.accessibleName} aria-describedby={item.accessibleName && item.hint ? `${descriptionId}-${item.value}` : undefined} aria-pressed={props.value === item.value} title={item.hint} aria-description={item.accessibleName ? undefined : item.hint} onClick={() => props.onChange(item.value)}>
          <Show when={item.icon}><Icon name={item.icon!} size={13} /></Show>
          {item.label}
          <Show when={item.count !== undefined}><span class="ui-tab-count">{item.count}</span></Show>
          <Show when={item.accessibleName && item.hint}><span hidden id={`${descriptionId}-${item.value}`}>{item.hint}</span></Show>
        </button>
      }</For>
    </nav>
  );
}

/** A segmented control: two to four mutually exclusive options. */
export function Segmented<T extends string>(props: {
  label: string;
  items: readonly { value: T; label: string }[];
  value: T;
  onChange: (value: T) => void;
  class?: string;
  level?: 'primary' | 'secondary';
}) {
  return (
    <div class={`ui-segmented${props.level ? ` ui-level-${props.level}` : ''}${props.class ? ` ${props.class}` : ''}`} role="group" aria-label={props.label}>
      <For each={props.items}>{item =>
        <button type="button" aria-pressed={props.value === item.value} onClick={() => props.onChange(item.value)}>{item.label}</button>
      }</For>
    </div>
  );
}

/** The one empty state: an optional icon, a title, a sentence, actions. */
export function EmptyState(props: {
  icon?: IconName;
  art?: { kind: EmptyScene; evidence: EmptyEvidence };
  title: JSX.Element;
  children?: JSX.Element;
  actions?: JSX.Element;
  class?: string;
}) {
  return (
    <div class={`ui-empty${props.class ? ` ${props.class}` : ''}`}>
      <Show when={props.art && emptyStateScene(props.art.kind, props.art.evidence)}
        fallback={<Show when={props.icon}><span class="ui-empty-icon"><Icon name={props.icon!} size={20} /></span></Show>}>
        {kind => <EmptyArt kind={kind()} evidence={{ ready: true }} compact />}
      </Show>
      <h2 class="ui-empty-title">{props.title}</h2>
      <Show when={props.children}><p class="ui-empty-body">{props.children}</p></Show>
      <Show when={props.actions}><div class="ui-empty-actions">{props.actions}</div></Show>
    </div>
  );
}

/** A list row: an optional leading icon (or any `leading` element, e.g. a
 * project icon), a title with a meta line, trailing content. Renders a button
 * when it has `onClick`. */
export function ListRow(props: {
  icon?: IconName;
  leading?: JSX.Element;
  title: JSX.Element;
  meta?: JSX.Element;
  trailing?: JSX.Element;
  selected?: boolean;
  disabled?: boolean;
  onClick?: (e: MouseEvent) => void;
  onContextMenu?: (e: MouseEvent) => void;
  class?: string;
  label?: string;
  /** Additional row facts when an explicit label replaces the visible contents. */
  description?: string;
  /** Full text on hover, for titles the row may cut short. */
  tooltip?: string;
  /** A row of a key list (`data-key-list`, five asks A4): j / k reach it. */
  keyItem?: boolean;
}) {
  const body = () => <>
    <Show when={props.leading ?? props.icon}><span class="ui-row-icon">{props.leading ?? <Icon name={props.icon!} size={15} />}</span></Show>
    <span class="ui-row-main"><span class="ui-row-title">{props.title}</span><Show when={props.meta}><span class="ui-row-meta">{props.meta}</span></Show></span>
    <Show when={props.trailing}><span class="ui-row-trailing">{props.trailing}</span></Show>
  </>;
  const cls = () => `ui-row${props.selected ? ' selected' : ''}${props.class ? ` ${props.class}` : ''}`;
  return (
    <Show when={props.onClick} fallback={<div class={cls()} onContextMenu={e => props.onContextMenu?.(e)}>{body()}</div>}>
      <button type="button" class={cls()} data-key-item={props.keyItem ? '' : undefined} aria-label={props.label} aria-description={props.description} title={props.tooltip} aria-pressed={props.selected} disabled={props.disabled}
        onClick={e => props.onClick!(e)} onContextMenu={e => props.onContextMenu?.(e)}>{body()}</button>
    </Show>
  );
}

const SKELETON_WIDTHS = ['72%', '94%', '58%', '86%', '64%', '90%'];

/** Placeholder bars while content loads (frontier F8), in place of a bare
 * "Loading…" line. Announced once through `label`; the bars are decorative. */
export function Skeleton(props: { rows?: number; label?: string; class?: string }) {
  const bars = () => Array.from({ length: props.rows ?? 3 }, (_, i) => SKELETON_WIDTHS[i % SKELETON_WIDTHS.length]);
  return (
    <div class={`ui-skeleton${props.class ? ` ${props.class}` : ''}`} role="status" aria-busy="true" aria-label={props.label ?? 'Loading'}>
      <For each={bars()}>{width => <span class="ui-skeleton-bar" style={{ width }} aria-hidden="true" />}</For>
    </div>
  );
}

/** A compact state word with a mark (next-level P01). The tone comes from the
 * shared vocabulary in `workState.ts` / `capabilityState.ts`, so a state
 * reads the same on every page. `hint` is the tooltip and the screen-reader
 * description; the colour never carries meaning on its own (the label does). */
export function StatusPill(props: { label: string; tone: string; hint?: string; class?: string; quiet?: boolean }) {
  return (
    <span class={`ui-pill${props.quiet ? ' quiet' : ''}${props.class ? ` ${props.class}` : ''}`} data-tone={props.tone} title={props.hint}>
      <Show when={props.tone === 'warn' || props.tone === 'error'} fallback={<i aria-hidden="true" />}>
        <Icon name={props.tone === 'error' ? 'x' : 'alert'} size={12} />
      </Show>{props.label}
      <Show when={props.hint}><span class="sr-only">. {props.hint}</span></Show>
    </span>
  );
}

/** A designed non-ready state for a whole surface (next-level P01): loading,
 * genuinely empty, filtered empty, offline, partial, blocked, failed,
 * conflicted or stale. One look everywhere, an honest title and sentence, and
 * the next action. `kind` picks the icon. */
export function StatePanel(props: {
  kind: import('../workState').SurfaceState;
  title: JSX.Element;
  children?: JSX.Element;
  actions?: JSX.Element;
  class?: string;
  compact?: boolean;
  visual?: JSX.Element;
  /** A failure's error: its plain sentence, then any children, then the raw
   *  text behind Details (`PlainErrorText`, overnight S5). */
  error?: unknown;
}) {
  const visual = children(() => props.visual);
  const icon = () => ({ loading: 'clock', empty: 'folder', filtered: 'search', offline: 'wifiOff', partial: 'alert', blocked: 'lock', failed: 'alert', conflicted: 'compare', stale: 'history' } as const)[props.kind];
  return (
    <div class={`ui-state${props.compact ? ' compact' : ''}${props.class ? ` ${props.class}` : ''}`} data-kind={props.kind}
      role={props.kind === 'failed' || props.kind === 'conflicted' ? 'alert' : 'status'} aria-busy={props.kind === 'loading' ? 'true' : undefined}>
      <Show when={visual()} fallback={<span class="ui-state-icon" aria-hidden="true"><Icon name={icon()} size={18} /></span>}>{visual()}</Show>
      <div class="ui-state-text">
        <strong class="ui-state-title">{props.title}</strong>
        <Show when={props.error != null || props.children}><p class="ui-state-body"><Show when={props.error != null} fallback={props.children}><PlainErrorText text={errMsg(props.error)}>{props.children}</PlainErrorText></Show></p></Show>
        <Show when={props.actions}><div class="ui-state-actions">{props.actions}</div></Show>
      </div>
    </div>
  );
}

/** An inline error (morning M6): the message in plain language, and the raw
 *  error behind a "Details" toggle when it was translated, as the snackbar
 *  shows it (overnight C7). Everything stays inside the one alert element as
 *  phrasing content (a `<details>` can't sit in a `<p>`), with the surface's
 *  own class. `plain={false}` shows the text as it is (a line that also
 *  carries warnings). */
export function ErrorText(props: { text: string; class: string; as?: 'p' | 'div'; plain?: boolean; children?: JSX.Element }) {
  const plain = createMemo(() => shownError(props.text, props.plain !== false));
  const body = () => <>{plain().text}{props.children}<ErrorDetails detail={plain().detail} /></>;
  return (
    <Show when={props.as === 'div'} fallback={<p class={props.class} role="alert">{body()}</p>}>
      <div class={props.class} role="alert">{body()}</div>
    </Show>
  );
}

/** The same plain sentence and Details toggle as `ErrorText`, without an
 *  element of its own (overnight S5): for a place that already is the alert
 *  or the error line, such as a `StatePanel` body, a list item or a line that
 *  has its own lead-in and buttons. `children` follow the sentence, ahead of
 *  Details. A note that is only sometimes an error passes `plain={false}`
 *  when it isn't, so a title the person chose is shown as it is. */
export function PlainErrorText(props: { text: string; plain?: boolean; children?: JSX.Element }) {
  const plain = () => shownError(props.text, props.plain !== false);
  return <>{plain().text}{props.children}<ErrorDetails detail={plain().detail} /></>;
}

/** The "Details" toggle with the raw error, as phrasing content; nothing
 *  when there is no raw error. Exported for a line whose raw error isn't one
 *  `plainError` recognises but still belongs under Details (B04: an
 *  automation's unreadable schedule), as an `ErrorText` child. */
export function ErrorDetails(props: { detail: string | null }) {
  const [open, setOpen] = createSignal(false);
  const id = createUniqueId();
  return (
    <Show when={props.detail}>{d =>
      <span class="inline-error-details">
        <button type="button" class="inline-error-toggle" aria-expanded={open()} aria-controls={id} onClick={() => setOpen(o => !o)}>Details</button>
        <code class="inline-error-detail" id={id} hidden={!open()}>{d()}</code>
      </span>}
    </Show>
  );
}

/** A labelled section of a page (next-level P01): a quiet heading with an
 * optional count and action, then content. Replaces cards-inside-cards with
 * separators and hierarchy. */
export function Section(props: { title: JSX.Element; count?: number | string; actions?: JSX.Element; children: JSX.Element; class?: string; id?: string; label?: string }) {
  return (
    <section class={`ui-section${props.class ? ` ${props.class}` : ''}`} id={props.id} aria-label={props.label}>
      <header class="ui-section-head">
        <h2 class="ui-section-title">{props.title}<Show when={props.count !== undefined && props.count !== ''}><span class="ui-section-count">{props.count}</span></Show></h2>
        <Show when={props.actions}><div class="ui-section-actions">{props.actions}</div></Show>
      </header>
      {props.children}
    </section>
  );
}
