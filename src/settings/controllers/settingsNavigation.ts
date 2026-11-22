// Settings' navigation controller (plan completion L4, X18: extracted from
// pages/Settings.tsx): which section is open, the search over sections and
// the controls inside them (audit H06), jumping to one control, and deep
// links from elsewhere (`requestOpenSettingsSection`). Called once by the
// Settings page, which owns the effects it sets up.
import { createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import { SETTINGS_SECTION_BY_ID, SETTINGS_SECTION_GROUPS, SETTINGS_SECTION_IDS, searchSettings, settingsPartHeading } from '../../settingsSections';
import { requestUsageTab, usageTabFor } from '../usageTabs';
import { resolveSettingsTarget } from '../../settingsResolve';
import { EXACT_GRACE_MS, JUMP_LIMIT_MS, keepInView, openFoldsAround, preferExact, retryFind } from '../../settingsJump';
import { pendingForPage, searchSettingsPages, type SettingsPageEntry } from '../pageTargets';
import { openSettingsPageTarget, setSettingsSectionRequest, settingsSectionRequest } from '../../store';
import { latestRequest } from '../../latestRequest';
import { motionReduced } from '../../motion';
import { observeKitMotion } from '../../motionKit';
import { clearSavedCue, dirtyLabels } from '../settingsDirty';
import { wantSettingsParts } from '../settingsLayout';

// The section open when Settings was last left, for this session (P1-21:
// Settings no longer always reopens on its first section, General since PR 3).
let rememberedSection = SETTINGS_SECTION_IDS[0];

/** A switch waiting on the unsaved-changes question (P1-21). */
export interface PendingSwitch { to: string; label?: string; unsaved: string[]; /** Set when the switch leaves Settings for another page's entry. */ page?: SettingsPageEntry }

export function createSettingsNavigation() {
  const indexGroups = createMemo(() => SETTINGS_SECTION_GROUPS.map(g => ({ ...g, items: [...g.items] })));
  const [activeSection, setActiveSectionSignal] = createSignal(rememberedSection);
  const setActiveSection = (id: string) => { rememberedSection = id; setActiveSectionSignal(id); };
  const [sectionQuery, setSectionQuery] = createSignal('');
  const [pendingSwitch, setPendingSwitch] = createSignal<PendingSwitch | null>(null);
  // Sections and the visible controls inside them that match the search
  // (audit H06: "motion" finds Interface → Motion).
  const searchHits = createMemo(() => searchSettings(sectionQuery()));
  const filteredIndexGroups = createMemo(() => {
    const hits = searchHits();
    return indexGroups().map(group => ({ ...group, items: group.items.filter(item => hits.some(h => h.section.id === item.id)) })).filter(group => group.items.length);
  });
  // Settings that live on another page (Library, Automations): found here, opened there.
  const pageHits = createMemo(() => searchSettingsPages(sectionQuery()));
  // Leaving for another page unmounts Settings' forms like a section switch does, so it asks the same way.
  const openPageHit = (entry: SettingsPageEntry) => {
    const pending = pendingForPage(entry, dirtyLabels());
    if (!pending) { entry.open(); return; }
    const from = document.activeElement;
    if (!pendingSwitch()) askedFrom = from instanceof HTMLElement && from !== document.body ? from : null;
    setPendingSwitch(pending);
  };
  // A link or deep link to a page target (Library, Skills; Automations windows) asks the same way a page hit does.
  // The target's control label stays with it (overnight 11-13 S1), so the page opens on that control.
  const openPageTarget = (target: { page: string; tab?: string; label?: string }) => openPageHit({
    label: target.label ?? target.tab ?? target.page, keywords: '', page: target.page === 'vault' ? 'Library' : 'Automations', tab: target.tab,
    open: () => void openSettingsPageTarget(target),
  });
  const hitControls = (id: string) => sectionQuery().trim() ? searchHits().find(h => h.section.id === id)?.controls ?? [] : [];
  /** Open a section and bring one visible control into view, briefly marked.
   *  A lazy panel's controls appear only once its chunk loads (a cold first
   *  open of Usage), so it keeps looking for a moment (morning P3). */
  let cancelJump: (() => void) | undefined;
  // The landed jump's hold on its target while the section fills in.
  let cancelHold: (() => void) | undefined;
  // Each jump's ticket: a later jump (even one still waiting for its two
  // frames) supersedes an earlier one.
  const jumps = latestRequest();
  // The found mark's timer too (#1332 review): Settings closing must not leave it running.
  onCleanup(() => { cancelJump?.(); cancelHold?.(); clearTimeout(foundTimer); });
  /** Open a section, or a control in it, asking first when the open section
   *  holds unsaved changes (its forms unmount on a switch). A retired id
   *  opens the section that holds it, on the control or part it became
   *  (P1-22, S2). */
  function jumpTo(id: string, label?: string) {
    const target = resolveSettingsTarget(id, label);
    // A control that moved to another page (PR 4) opens that page.
    if (target.kind === 'page') { openPageTarget(target); return; }
    const section = target.section;
    label = target.label;
    const unsaved = section === activeSection() ? [] : dirtyLabels();
    if (unsaved.length) {
      // Keep editing returns focus here (the control that asked to switch).
      // WKWebView doesn't focus a clicked button, so a mouse switch leaves
      // <body> active: that counts as nothing, and the heading takes focus.
      const from = document.activeElement;
      if (!pendingSwitch()) askedFrom = from instanceof HTMLElement && from !== document.body ? from : null;
      setPendingSwitch({ to: section, label, unsaved });
      return;
    }
    go(section, label);
  }
  let askedFrom: HTMLElement | null = null;
  let foundRow: HTMLElement | undefined;
  let foundTimer: ReturnType<typeof setTimeout> | undefined;
  let stopFoundMotion: (() => void) | undefined;
  onCleanup(() => stopFoundMotion?.());
  /** The unsaved-changes question's two answers. */
  function keepEditing() {
    setPendingSwitch(null);
    const back = askedFrom?.isConnected ? askedFrom : document.querySelector<HTMLElement>(`#settings-${activeSection()} .settings-section-title`);
    askedFrom = null;
    queueMicrotask(() => back?.focus({ preventScroll: true }));
  }
  function leaveWithoutSaving() {
    const p = pendingSwitch();
    setPendingSwitch(null);
    if (p?.page) { askedFrom = null; p.page.open(); } else if (p) go(p.to, p.label);
  }
  function go(section: string, label?: string) {
    const switched = section !== activeSection();
    setPendingSwitch(null);
    askedFrom = null;
    // A Saved cue belongs to the section it was said in.
    if (switched) clearSavedCue();
    // The part a control sits in (S2): it, and every part above it, mounts
    // before the section renders, so a lazy part's rows are there to find
    // and nothing above them grows after the scroll.
    const meta = SETTINGS_SECTION_BY_ID[section];
    const inPart = label ? meta?.controls?.find(c => c.label === label)?.part : undefined;
    const parts = meta?.parts?.map(p => p.id) ?? [];
    wantSettingsParts(inPart ? parts.slice(0, parts.indexOf(inPart) + 1) : []);
    // Usage's tabs (Settings PR 8): a control on another tab opens that tab.
    if (section === 'usage' && label) requestUsageTab(usageTabFor(label));
    setActiveSection(section);
    cancelJump?.();
    cancelHold?.();
    const gen = jumps.start();
    if (!label) {
      // A new section starts at its top with its heading focused, so a
      // screen reader says where it landed (P1-21).
      if (switched) requestAnimationFrame(() => {
        if (!jumps.isLatest(gen)) return;
        document.querySelector('.settings-body')?.scrollTo({ top: 0 });
        document.querySelector<HTMLElement>(`#settings-${section} .settings-section-title`)?.focus({ preventScroll: true });
      });
      return;
    }
    // A part's heading (S2) lands on the part by its id, never by text: a
    // name like "Chat" starts other rows too ("Chats at once").
    const part = settingsPartHeading(section, label);
    // Shown instead when the control isn't on screen (a row that appears
    // only once its switch is on, P1-21).
    const near = SETTINGS_SECTION_BY_ID[section]?.controls?.find(c => c.label === label)?.near;
    // Focus moved elsewhere while the panel loaded: scroll, but don't take it.
    const focusedAtStart = document.activeElement;
    // `summary` (a disclosure such as Usage's Prices) and
    // `[data-settings-control]` (a control named only by its aria-label,
    // such as a switch beside bold text) are jump targets too (P1-21).
    const candidates = () => {
      // A control inside a part is looked for there (S2): "Claude" in
      // Accounts, never Agents' earlier "Claude model" row.
      const body = (inPart && document.getElementById(`settings-${inPart}`)) || document.querySelector('.settings-body');
      const named = (text: string) => (el: HTMLElement) => (el.dataset.settingsControl ?? (el.textContent ?? '').replace(/\s+/g, ' ').trim()).startsWith(text);
      // `[data-jump-skip]`: links to a control (Usage's jump links, P1-22) are never its target.
      return { named, all: [...(body?.querySelectorAll<HTMLElement>('.settings-row-k, .settings-subhead-title, h3, h4, label, button, summary, [data-settings-control]') ?? [])].filter(el => !el.closest('[data-jump-skip]')) };
    };
    // `.usage-block`: a Usage panel (Value this month, Routing) is a div in
    // the Usage section, which `section` alone would centre whole (G3).
    // `.budgets-fixed-row`: a cap Budgets keeps for you (P1-07), one row of the Budgets section.
    // Never the whole Settings section: a heading or button outside any row
    // is marked itself (a row-k with its description) instead (P1-21).
    // Never a `.settings-fold` either: a fold holds many rows, so a target
    // inside one (Launch calls) lands on its own row and opens the fold.
    const rowOf = (target: HTMLElement | undefined) => target && (target.closest<HTMLElement>('.settings-row, .settings-subhead, .usage-block, .budgets-fixed-row, details:not(.settings-fold), section:not(.settings-section), .about-you-block')
      ?? (target.matches('.settings-row-k') ? target.parentElement ?? target : target));
    // A control's own row first: an explainer card or heading earlier in the
    // section may share its name (the background-work card's "Main agent
    // check-ins" sits above the switch, A07), and on a cold open the row can
    // render after the card, so the card wins only once the row hasn't come.
    // A control that isn't shown lands on its `near` control.
    const find = part ? () => document.querySelector<HTMLElement>(`#settings-${part} > .settings-part-head`) : preferExact<HTMLElement>(
      () => { const { named, all } = candidates(); return rowOf(all.find(el => el.matches('.settings-row-k') && named(label)(el))); },
      () => { const { named, all } = candidates(); return rowOf(all.find(named(label)) ?? (near ? all.find(el => el.matches('.settings-row-k') && named(near)(el)) ?? all.find(named(near)) : undefined)); },
      EXACT_GRACE_MS,
    );
    const land = (row: HTMLElement) => {
      // A row in a hidden tab panel (Keyboard shortcuts) selects its tab first.
      for (let panel = row.closest<HTMLElement>('[role="tabpanel"][hidden]'); panel; panel = row.closest<HTMLElement>('[role="tabpanel"][hidden]')) {
        const tab = document.getElementById(panel.getAttribute('aria-labelledby') ?? '');
        if (!tab) break;
        tab.click();
        if (panel.hidden) break;
      }
      // A row inside a closed fold (an Advanced group, PR 3) opens it; a row
      // that is itself a disclosure stays as it was.
      openFoldsAround(row);
      // A part starts at the top, so what it holds reads below its heading.
      row.scrollIntoView({ block: part ? 'start' : 'center', behavior: motionReduced() ? 'auto' : 'smooth' });
      stopFoundMotion?.();
      row.classList.remove('settings-found'); void row.offsetWidth; row.classList.add('settings-found');
      stopFoundMotion = observeKitMotion(row, () => {});
      // One timer for the mark: an earlier jump's timer must not clear this jump's mark on the same row
      // (Move on at, then Resume on another account, share Spread usage's block).
      clearTimeout(foundTimer);
      if (foundRow !== row) foundRow?.classList.remove('settings-found');
      foundRow = row;
      foundTimer = setTimeout(() => { row.classList.remove('settings-found'); stopFoundMotion?.(); stopFoundMotion = undefined; }, 1800);
      if (document.activeElement === focusedAtStart || document.activeElement === document.body) {
        // The row's first control on screen (not one inside a closed
        // disclosure); a row whose only control is a disclosure (System's
        // "Folders agents can run tools from", A12) focuses its summary.
        // (`checkVisibility`: a closed <details> skips its content with
        // content-visibility, which still has client rects in Chromium.)
        // A row may name the control a jump means (`data-jump-focus`: the
        // Plaud row's Connect, P1-20) ahead of others it shows first.
        // A target that is itself a control (a button or disclosure outside
        // any row) takes focus. A part's heading takes it itself (S2), so a
        // screen reader says which part it landed on.
        const shown = (sel: string) => [...row.querySelectorAll<HTMLElement>(sel)].find(el => (el.checkVisibility ? el.checkVisibility() : el.getClientRects().length > 0));
        const heading = part ? row.querySelector<HTMLElement>('.settings-subhead-title') ?? undefined : undefined;
        (heading ?? shown('[data-jump-focus]') ?? shown('button, input, select, [role="combobox"]') ?? shown('summary') ?? (row.matches('button, summary, input, select') ? row : undefined))?.focus({ preventScroll: true });
      }
      // Content above the row may still fill in (a lazy part mounting above
      // it on a cold jump, S2 review) and push it away: WKWebView has no
      // scroll anchoring, and `.settings-body` sets `overflow-anchor: none`
      // so Chromium (the smoke suite) behaves the same. So for a moment each
      // size change of the section scrolls the row back, until the person
      // scrolls, presses a key, points or moves focus.
      const sectionEl = row.closest<HTMLElement>('.settings-section');
      if (!sectionEl || typeof ResizeObserver === 'undefined') return;
      const focusedNow = document.activeElement;
      cancelHold = keepInView({
        onResize: changed => {
          let height = sectionEl.getBoundingClientRect().height;
          const ro = new ResizeObserver(() => { const h = sectionEl.getBoundingClientRect().height; if (h !== height) { height = h; changed(); } });
          ro.observe(sectionEl);
          return () => ro.disconnect();
        },
        onInterrupt: stop => {
          const events = ['wheel', 'touchstart', 'pointerdown', 'keydown'] as const;
          const moved = () => { if (document.activeElement !== focusedNow) stop(); };
          for (const e of events) window.addEventListener(e, stop, { capture: true, passive: true });
          document.addEventListener('focusin', moved, true);
          return () => { for (const e of events) window.removeEventListener(e, stop, { capture: true }); document.removeEventListener('focusin', moved, true); };
        },
        rescroll: () => { if (row.isConnected) row.scrollIntoView({ block: part ? 'start' : 'center', behavior: 'auto' }); },
      });
    };
    // Two frames first so the section has rendered; then looking again
    // until the control appears, while this section is still the one open.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (!jumps.isLatest(gen)) return;
      // The limit adds the grace, so a fallback that first appears late still lands.
      cancelJump = retryFind(find, land, () => jumps.isLatest(gen) && activeSection() === section, { limit: JUMP_LIMIT_MS + EXACT_GRACE_MS });
    }));
  }
  // The alias-aware opener: use for any id that may predate the section
  // consolidation (capability-doctor fixSection values, deep links). It asks
  // first about unsaved changes, like every switch, and a retired id lands
  // on the control or part it became (jumpTo applies the alias control).
  const openSection = (id: string) => jumpTo(id);
  /** Enter in the search opens its top hit: the first matching control, or
   *  the first matching section (P1-21). */
  function openTopHit() {
    if (!sectionQuery().trim()) return;
    const [first] = searchHits();
    if (first) jumpTo(first.section.id, first.controls[0]?.label);
    else if (pageHits()[0]) openPageHit(pageHits()[0]);
  }

  // If the active section id ever stops being real (e.g. a stale persisted
  // id from before a consolidation), fall back to the first section so the
  // body is never blank.
  createEffect(() => {
    const flat = indexGroups().flatMap(g => g.items);
    if (flat.length && !flat.some(s => s.id === activeSection())) {
      setActiveSection(flat[0].id);
    }
  });

  // Cross-page deep-link: a notification (e.g. the heartbeat approval-queue
  // "Review" CTA, or the Rust-emitted `settings:heartbeat` action URL) can
  // request a specific section. Old ids resolve through the alias map so
  // pre-consolidation links keep landing; unknown ids are dropped so we
  // never show a dead target. The request is consumed so it doesn't
  // "stick" and override the default section on a later remount.
  createEffect(() => {
    const req = settingsSectionRequest();
    if (!req) return;
    const target = resolveSettingsTarget(req.id, req.control);
    const id = target.kind === 'section' ? target.section : '';
    const flat = indexGroups().flatMap(g => g.items);
    // A retired section lands on the control or part it became (P1-22, S2);
    // a control that moved to another page goes through jumpTo too, so it
    // asks about unsaved changes first.
    if (target.kind === 'page' || flat.some(s => s.id === id)) jumpTo(req.id, req.control);
    setSettingsSectionRequest(null);
  });

  return { activeSection, sectionQuery, setSectionQuery, indexGroups, filteredIndexGroups, hitControls, pageHits, openPageHit, openPageTarget, jumpTo, openSection, openTopHit, pendingSwitch, keepEditing, leaveWithoutSaving };
}
