// Settings page layout parts (plan completion L4, X18: extracted from
// pages/Settings.tsx): the active-section context, a section with its
// scope line, a part (a former section inside one, S2), a sub-header and a
// key/value row.
import { Show, createContext, createEffect, createSignal, onCleanup, onMount, useContext, type JSX } from 'solid-js';
import { SETTINGS_SECTION_BY_ID, saveNote } from '../settingsSections';
import { RecoverableView } from '../components/RecoverableView';
import { Skeleton } from '../components/ui';
import { savedCue } from './settingsDirty';
import { splitLead } from './leadText';
import { AnimatedHeight } from '../components/MotionKit';

/// Long help text with its first sentence in view and the rest one visible
/// click away, on a "More" that says "Less" once open (overnight 11-07). All
/// of it stays in the page: the rest sits in a closed `details`, so a search
/// or a screen reader still reaches it and nothing is dropped. A short text,
/// or one with no sentence to cut at, is shown whole (`splitLead`).
export function LeadText(props: { text: string; class?: string }) {
  const cut = () => splitLead(props.text);
  return (
    <Show when={cut()} fallback={<p class={props.class}>{props.text}</p>}>
      {parts => (
        <>
          <p class={props.class}>{parts()[0]}</p>
          <MoreFold><p class={props.class}>{parts()[1]}</p></MoreFold>
        </>
      )}
    </Show>
  );
}

/// The rest of a help text, closed behind a visible "More" that reads "Less"
/// once open: a row writes its first sentence, then this with the remainder.
export function MoreFold(props: { children: JSX.Element }) {
  return (
    <AnimatedHeight><details class="settings-fold settings-row-more settings-lead-more">
      <summary><span class="lead-more-show">More</span><span class="lead-more-hide">Less</span></summary>
      {props.children}
    </details></AnimatedHeight>
  );
}

/// The currently-selected section id, provided by `Settings` so each
/// `SettingsSection` renders only when it's the active menu choice (true
/// section switching, not a long scroll).
export const ActiveSectionCtx = createContext<() => string>(() => 'general');

/// One Settings section: a title and an optional status chip over the row
/// body (frontier F3: no icon tile, no card). It has no hint of its own since
/// Settings PR 6: the section's name says it. Only the active section
/// renders; `icon` is used by the section list.
export function SettingsSection(props: {
  id: string;
  icon: import('../icons').IconName;
  title: string;
  status?: JSX.Element;
  children: any;
}) {
  const active = useContext(ActiveSectionCtx);
  // What it reaches, when a change takes hold and how it saves (L4, X11).
  const scope = () => SETTINGS_SECTION_BY_ID[props.id]?.scope;
  return (
    <Show when={active() === props.id}>
    <section class="settings-section" id={`settings-${props.id}`}>
      <header class="settings-section-head">
        <div class="settings-section-titles">
          {/* Focused on a section switch (P1-21), so it can take focus. */}
          <h2 class="settings-section-title" tabindex="-1">{props.title}</h2>
          <Show when={scope()}>{sc => (
            <p class="settings-section-scope settings-save-note" role="note" aria-label={`What ${props.title} changes`}>{saveNote(sc())}</p>
          )}</Show>
        </div>
        <Show when={props.status}>
          <div class="settings-section-status">{props.status}</div>
        </Show>
        {/* "Saved" for a control that saves as you change it (P1-21); the
            live region stays mounted so each change is announced. */}
        <p class="settings-saved-cue" aria-live="polite">{savedCue()}</p>
      </header>
      <div class="settings-section-body"><RecoverableView name={props.title} compact fallback={<Skeleton label={`Loading ${props.title}`} rows={3} />}>{props.children}</RecoverableView></div>
    </section>
    </Show>
  );
}

/// The parts a jump or search is headed for (S2): a lazy part among them
/// mounts at once, before the jump looks for a row inside it. Navigation
/// names the target part and every part above it in the section, so a part
/// that grows above the target can't push it away after the scroll. An
/// empty set on a plain section switch.
const [wantedParts, setWantedParts] = createSignal<ReadonlySet<string>>(new Set());
export const wantSettingsParts = (ids: readonly string[]) => setWantedParts(new Set(ids));

/// How far outside the visible Settings body a lazy part starts mounting.
const NEAR_MARGIN = '800px 0px';

/// A lazy part's body (S2 review): mounted the first time it comes within
/// NEAR_MARGIN of the visible Settings body, or when a jump wants it, then
/// kept. Until then a placeholder holds its place, so opening a section
/// doesn't read History's runs, the Skills catalog or every account at once.
function MountWhenNear(props: { id: string; children: JSX.Element }) {
  const wanted = () => wantedParts().has(props.id);
  const [near, setNear] = createSignal(wanted() || typeof IntersectionObserver === 'undefined');
  let marker: HTMLDivElement | undefined;
  let io: IntersectionObserver | undefined;
  // Mounted, by coming near or by a jump wanting it first: nothing left to watch.
  createEffect(() => { if (wanted()) setNear(true); if (near()) { io?.disconnect(); io = undefined; } });
  onMount(() => {
    if (near()) return;
    // One frame on, so the placeholder sits in the scrolling body (its root).
    const frame = requestAnimationFrame(() => {
      if (near() || !marker) return;
      io = new IntersectionObserver(entries => { if (entries.some(e => e.isIntersecting)) setNear(true); },
        { root: marker.closest('.settings-body'), rootMargin: NEAR_MARGIN });
      io.observe(marker);
    });
    onCleanup(() => { cancelAnimationFrame(frame); io?.disconnect(); });
  });
  return <Show when={near()} fallback={<div ref={marker} class="settings-part-pending" aria-hidden="true" />}>{props.children}</Show>;
}

/// A lazy closed group's body: mounted once its fold has opened or a jump
/// wants it, then kept. Nothing stands in for it while the fold is closed.
function MountWhenOpened(props: { id: string; opened: boolean; children: JSX.Element }) {
  const [mounted, setMounted] = createSignal(false);
  createEffect(() => { if (props.opened || wantedParts().has(props.id)) setMounted(true); });
  return <Show when={mounted()}>{props.children}</Show>;
}

/// A former section kept whole inside a section (S2): its heading, and its
/// body in its own recovery boundary, so one part that fails to load leaves
/// the rest of the section readable. A `div`, never a `section`: the jump
/// treats a nested `section` as one row. Its id stays the old section's
/// (`settings-<id>`), so old links, styles and specs still find it. `lazy`
/// mounts the body only once it comes near the screen or a jump wants it.
export function SettingsPart(props: { id: string; title: string; hint?: string; status?: JSX.Element; lazy?: boolean; group?: boolean; children: any }) {
  const active = useContext(ActiveSectionCtx);
  const note = () => saveNote(SETTINGS_SECTION_BY_ID[active()]?.scope);
  const body = () => <RecoverableView name={props.title} compact fallback={<Skeleton label={`Loading ${props.title}`} rows={3} />}>{props.children}</RecoverableView>;
  // A part that is also one of Advanced's closed groups (PR 3): its heading
  // is the group's summary, so a jump to the part opens the group.
  // A lazy group mounts its body when the fold first opens (a click, or a
  // jump that opens it) or a jump wants it, never by coming near: a closed
  // fold's content is not rendered, so it never intersects anything and the
  // near-placeholder would wait forever (Sources' Context, CX17).
  if (props.group) {
    const [opened, setOpened] = createSignal(false);
    return (
      <details class="settings-fold settings-group settings-part" id={`settings-${props.id}`} on:toggle={e => { if (e.currentTarget.open) setOpened(true); }}>
        <summary class="settings-subhead settings-part-head"><GroupTitles title={props.title} hint={props.hint} part />
          <Show when={props.status}><div class="settings-section-status">{props.status}</div></Show>
        </summary>
        {props.lazy ? <MountWhenOpened id={props.id} opened={opened()}>{body()}</MountWhenOpened> : body()}
      </details>
    );
  }
  return (
    <div class="settings-part" id={`settings-${props.id}`}>
      <SettingsSubhead title={props.title} hint={props.hint} status={props.status} part />
      <p class="settings-part-save settings-save-note">{note()}</p>
      {props.lazy ? <MountWhenNear id={props.id}>{body()}</MountWhenNear> : body()}
    </div>
  );
}

/// One of Advanced's closed groups (Settings PR 3, plan 3.3): a heading
/// that opens to its rows. A search or jump to a row inside opens it.
export function SettingsGroup(props: { id: string; title: string; hint?: string; children: any }) {
  return (
    <details class="settings-fold settings-group" id={`settings-group-${props.id}`}>
      <summary class="settings-subhead"><GroupTitles title={props.title} hint={props.hint} /></summary>
      <RecoverableView name={props.title} compact fallback={<Skeleton label={`Loading ${props.title}`} rows={3} />}>{props.children}</RecoverableView>
    </details>
  );
}

/// A group's title and its one line, inside its summary.
function GroupTitles(props: { title: string; hint?: string; part?: boolean }) {
  return (
    <div class="settings-subhead-titles">
      <h3 class="settings-subhead-title" tabindex={props.part ? '-1' : undefined}>{props.title}</h3>
      <Show when={props.hint}>{hint => <p class="settings-section-hint">{hint()}</p>}</Show>
    </div>
  );
}

/// Part of a part mounted only when near (History & recovery's run list,
/// S2 review), for a part whose other rows should stay at hand.
export function SettingsLazy(props: { part: string; children: JSX.Element }) {
  return <MountWhenNear id={props.part}>{props.children}</MountWhenNear>;
}

/// A sub-header inside a consolidated section: carries the title (and,
/// optionally, the hint + status chip) of a former standalone section so
/// merged bodies keep their identity without re-scattering the index. A
/// part's heading (`part`) takes focus when a jump lands on the part.
export function SettingsSubhead(props: { title: string; hint?: string; status?: JSX.Element; part?: boolean }) {
  return (
    <div class={props.part ? 'settings-subhead settings-part-head' : 'settings-subhead'}>
      <div class="settings-subhead-titles">
        <h3 class="settings-subhead-title" tabindex={props.part ? '-1' : undefined}>{props.title}</h3>
        <Show when={props.hint}>{hint => <LeadText text={hint()} class="settings-section-hint" />}</Show>
      </div>
      <Show when={props.status}>
        <div class="settings-section-status">{props.status}</div>
      </Show>
    </div>
  );
}

export function KV(props: { k: string; v: string; warn?: string; mono?: boolean; action?: JSX.Element }) {
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">{props.k}</div>
        <div class={`settings-row-v ${props.warn ? 'warn' : ''} ${props.mono ? 'mono-v' : ''}`}>{props.v}</div>
        <Show when={props.warn}>
          <div class="settings-row-warn">{props.warn}</div>
        </Show>
      </div>
      {props.action}
    </div>
  );
}
