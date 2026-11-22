import { inSelectPopover } from './Select';
/**
 * The composer's model menu: one searchable list of every installed agent's
 * models, grouped by agent, with the current one checked. Picking a model
 * from another agent switches the chat's agent (switchChatAgent, which
 * also clears the native session id, as the old Agent select did, and moves
 * the chat's permission mode to the new agent's nearest one, saying so).
 * The Effort row sets this chat's reasoning effort only (Settings PR 7); the
 * app default is Settings, Models, Effort. Edit opens Settings.
 * Its last option, Ultracode, is this chat's switch instead (merged plan
 * Track U): it is never saved as a level, and picking a saved level turns it
 * off. The Canvases row (Canvas C3 wave 2 C4) is this chat's own choice of
 * how often its agent answers with a canvas, or "Use the setting" (Settings
 * → Chat → Canvases); it travels with the conversation.
 *
 * The Permissions row (2026-11-01) replaces the composer's separate
 * permissions button: one menu holds everything about how this chat runs.
 * The trigger's name still says the mode ("… · Permissions: Bypass all
 * (unrestricted)"); the words moved to the composer's access chip
 * (overnight 11-12, C3), and the trigger moved to the bar's right, before
 * the mic, so its list opens from the right.
 * A change goes through confirmPermissionModeChange like every other mode
 * picker, and waits while a turn runs.
 */
import { For, Show, Suspense, lazy, createEffect, createMemo, createSignal, onCleanup, untrack } from 'solid-js';
import { api } from '../api';
import type { AgentCliStatus } from '../generated/ipc';
import { latestRequest } from '../latestRequest';
import { chatSteerSupport } from '../store/chatRuntime';
import { syncAgentHealthSupport } from '../store/agentHealth';
import { Icon } from '../icons';
import { AgentMark } from './AgentMark';
import { createExitPhase } from './exit';
import { loadCatalog } from '../modelCatalog';
import { customRows, effortLabel, levelsForModel, effortShown, focusLeavesMenu, menuGroups, modelLabel, rowIndexByKey, rowKey, sameRow, type MenuModel, type MenuRow } from '../modelMenu';
import { ultracodeNote } from '../ultracode';
import {
  chatModel, setChatModelPersistent, chatBusy, chatSessionLoading, chatAgent,
  AGENT_LABELS, agentsAvailable, agentDescriptors, agentSettings, chatEffort, setChatEffortTracked, restoreChatEffortChoice, chatAgentSessionKey,
  openSettingsFromOutside, modelMenuRequested, setModelMenuRequested, type ChatAgent,
  chatUltracode, setChatUltracodePersistent, chatCanvases, setChatCanvasesPersistent,
  chatSessionId, chatTurns,
  chatPermissionMode, setChatPermissionModePersistent, switchChatAgent,
  confirmPermissionModeChange, currentAgentCapabilities, sameChatAsNow, flashFooter, type ChatPermissionMode,
} from '../store';
import { permModeNote, permModeOptions, permModeUnavailable, permissionsTriggerLabel } from '../permModes';
import { agentModeDanger, agentModeDescriptions, agentModeLabels, agentModeOrder, hollowCardsLine, shownMode } from '../agentPermModes';
import { MIN_ROOM, clipTopsAbove, roomAbove } from '../popoverRoom';
import { CANVAS_FREQUENCIES, CANVAS_FREQUENCY_LABELS, canvasesMenuValue, settingCanvases } from '../canvasFrequency';
import '../styles/chat-model-menu.css';

const ModelMenuHealth = lazy(() => import('./ModelMenuHealth'));

export function ChatModelMenu() {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal('');
  const [effortOpen, setEffortOpen] = createSignal(false);
  const [canvasesOpen, setCanvasesOpen] = createSignal(false);
  const [permOpen, setPermOpen] = createSignal(false);
  const canvasSetting = () => settingCanvases(agentSettings());
  const [catalogs, setCatalogs] = createSignal<Record<string, MenuModel[] | undefined>>({});
  const [warnings, setWarnings] = createSignal<Record<string, string | undefined>>({});
  const [loading, setLoading] = createSignal<Record<string, boolean>>({});
  const [health, setHealth] = createSignal<AgentCliStatus[]>([]);
  const [healthError, setHealthError] = createSignal(false);
  const [healthRetry, setHealthRetry] = createSignal(0);
  const reads = new Map<string, ReturnType<typeof latestRequest>>();
  const phase = createExitPhase(open, 120);
  let root: HTMLDivElement | undefined;
  let search: HTMLInputElement | undefined;
  let permRow: HTMLButtonElement | undefined;
  let effortRow: HTMLButtonElement | undefined;
  let canvasesRow: HTMLButtonElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  // A press on the button toggles the menu itself: the focus it takes on the
  // way must not close the menu first (ENG-81).
  let pressingTrigger = false;
  // How tall the popover may open above the trigger (popoverRoom.ts):
  // measured when it opens, up to the nearest pane that clips.
  const [room, setRoom] = createSignal<number | null>(null);

  const locked = () => chatBusy() || chatSessionLoading();
  const agents = () => agentsAvailable().map(id => ({ id, label: AGENT_LABELS[id] }));
  const levels = (agent: string) => agentDescriptors().find(d => d.id === agent)?.capabilities.effortLevels ?? [];
  // The chat's model may take fewer levels than its agent (Antigravity's Argon has no Extra high).
  const chatLevels = () => levelsForModel(levels(chatAgent()), catalogs()[chatAgent()], chatModel());
  const effortOf = (agent: string) => (levels(agent).length ? (agent === chatAgent() ? chatEffort() ?? undefined : (agentSettings().efforts ?? {})[agent]) : undefined);

  // Load every installed agent's catalog once (cached for the session).
  function load(agent: string, refresh = false) {
    if (!refresh && (catalogs()[agent] || loading()[agent])) return;
    const order = reads.get(agent) ?? latestRequest();
    reads.set(agent, order);
    const ticket = order.start();
    setLoading(prev => ({ ...prev, [agent]: true }));
    void loadCatalog(agent, refresh).then(c => {
      if (!order.isLatest(ticket)) return;
      setCatalogs(prev => ({ ...prev, [agent]: c.models }));
      setWarnings(prev => ({ ...prev, [agent]: c.warning ?? undefined }));
      setLoading(prev => ({ ...prev, [agent]: false }));
    }, error => {
      if (!order.isLatest(ticket)) return;
      setWarnings(prev => ({ ...prev, [agent]: String(error) }));
      setLoading(prev => ({ ...prev, [agent]: false }));
    });
  }
  createEffect(() => { const agents = agentsAvailable(); untrack(() => agents.forEach(agent => load(agent))); });
  onCleanup(() => reads.forEach(order => order.cancel()));
  createEffect(() => {
    if (!open()) return;
    healthRetry();
    const order = latestRequest();
    let pending = false;
    let ticks = 0;
    const refresh = () => {
      if (pending) return;
      pending = true;
      const ticket = order.start();
      void api.agentCliStatus(false).then(rows => {
        if (!order.isLatest(ticket)) return;
        setHealthError(false); setHealth(rows); syncAgentHealthSupport(rows);
      }, () => { if (order.isLatest(ticket)) setHealthError(true); }).finally(() => { pending = false; });
    };
    refresh();
    const timer = setInterval(() => {
      ticks++;
      if (health().some(row => row.state !== 'idle') || ticks % 6 === 0) refresh();
    }, 5000);
    onCleanup(() => { clearInterval(timer); order.cancel(); });
  });

  const current = () => ({ agent: chatAgent(), model: chatModel() });
  const groups = createMemo(() => menuGroups(agents(), catalogs(), current(), query()));
  const extras = createMemo(() => customRows(agents(), catalogs(), query()));
  const flat = createMemo<MenuRow[]>(() => [...groups().flatMap(g => g.rows), ...extras()]);
  // The highlight is the row itself, not its place (ENG-81): rows a catalog
  // adds while the menu is open don't move it to another model.
  const [activeKey, setActiveKey] = createSignal<string | null>(null);
  const active = createMemo(() => rowIndexByKey(flat(), activeKey()));
  const setActive = (index: number) => { const row = untrack(flat)[index]; setActiveKey(row ? rowKey(row) : null); };

  createEffect(() => { query(); setActive(0); });
  createEffect(() => {
    if (!open()) return;
    // Runs once per opening: reading the rows must not re-run it per keystroke.
    untrack(() => {
      setQuery('');
      setEffortOpen(false);
      setCanvasesOpen(false);
      setPermOpen(false);
      setActive(Math.max(0, flat().findIndex(r => sameRow(r, current(), catalogs()[r.agent]))));
    });
    // A hidden trigger (the palette's Switch model… over another view)
    // measures nothing; the CSS budget stands until it is shown.
    const measure = () => { const r = trigger?.getBoundingClientRect(); setRoom(trigger && r && r.height > 0 ? Math.max(MIN_ROOM, roomAbove(r.top, clipTopsAbove(trigger))) : null); };
    measure();
    window.addEventListener('resize', measure);
    onCleanup(() => window.removeEventListener('resize', measure));
    queueMicrotask(() => search?.focus());
    const onDown = (e: PointerEvent) => { if (root && !root.contains(e.target as Node) && !inSelectPopover(e.target)) setOpen(false); };
    document.addEventListener('pointerdown', onDown, true);
    // Cmd+Tab to another app closes it too (ENG-81).
    const onBlur = () => setOpen(false);
    window.addEventListener('blur', onBlur);
    onCleanup(() => { document.removeEventListener('pointerdown', onDown, true); window.removeEventListener('blur', onBlur); });
  });

  // "Switch model…" in the palette (frontier F6) asks the composer's menu to open.
  createEffect(() => {
    if (!modelMenuRequested()) return;
    setModelMenuRequested(false);
    if (agentsAvailable().length) queueMicrotask(() => setOpen(true));
  });

  function choose(row: MenuRow) {
    if (locked()) return;
    if (row.agent !== chatAgent()) switchChatAgent(row.agent as ChatAgent);
    setChatModelPersistent(row.id);
    setOpen(false);
  }

  // This chat only (Settings PR 7): the app default is Settings, Models, Effort.
  // A failed save puts the prior choice back only while it is still this
  // chat's newest pick (a later pick wins), and by the chat's key, so a
  // failure after a switch restores that chat, not the one on screen.
  async function setEffort(level: string) {
    const key = chatAgentSessionKey(), wasUltra = chatUltracode(), was = chatEffort();
    // A level is a choice to leave this chat's ultracode.
    setChatUltracodePersistent(false);
    const saved = setChatEffortTracked(level || null);
    setEffortOpen(false);
    effortRow?.focus();
    if (await saved === 'failed-latest' && key) {
      restoreChatEffortChoice(key, was, wasUltra);
      flashFooter("Couldn't save the effort level. It is unchanged.");
    }
  }

  // The chat's agent's own modes (per-agent menus, 2026-11-12): the stored
  // mode as that agent's option shows it, in its own words.
  const permMode = () => shownMode(chatAgent(), chatPermissionMode());
  const permLabels = () => agentModeLabels(chatAgent());
  const permDanger = () => agentModeDanger(chatAgent())[permMode()];
  async function setPermission(next: ChatPermissionMode) {
    if (next === permMode() || locked() || permModeUnavailable(currentAgentCapabilities(), next)) return;
    // The confirm's answer belongs to the chat it was asked on.
    const still = sameChatAsNow();
    const ok = await confirmPermissionModeChange(next, chatAgent());
    if (!ok || locked() || !still()) return;
    setChatPermissionModePersistent(next);
    setPermOpen(false);
    // The list closes under the focused option: hand focus to its row.
    queueMicrotask(() => permRow?.focus());
  }

  /** Folds a setting list back onto its row and focuses the row. */
  function foldSetting(which: string | undefined) {
    if (which === 'permissions') { setPermOpen(false); permRow?.focus(); }
    else if (which === 'effort') { setEffortOpen(false); effortRow?.focus(); }
    else if (which === 'canvases') { setCanvasesOpen(false); canvasesRow?.focus(); }
  }

  function onKey(e: KeyboardEvent) {
    const button = e.target instanceof Element ? e.target.closest('button') : null;
    if (button && (e.key === 'Enter' || e.key === ' ')) return;
    // The setting lists (Effort, Permissions, Canvases; 2026-11-01 review):
    // arrows, Home and End move between their options, as the Select the
    // permissions button used did, and Escape folds the list onto its row.
    const setting = button?.closest<HTMLElement>('.model-menu-efforts[data-setting]');
    if (setting && button) {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); foldSetting(setting.dataset.setting); return; }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End') {
        e.preventDefault();
        const options = Array.from(setting.querySelectorAll<HTMLButtonElement>('[role="option"]:not(:disabled)'));
        const at = options.indexOf(button);
        const next = e.key === 'Home' ? 0 : e.key === 'End' ? options.length - 1
          : Math.min(options.length - 1, Math.max(0, at + (e.key === 'ArrowDown' ? 1 : -1)));
        options[next]?.focus();
        return;
      }
    }
    if (e.key !== 'Escape' && button && (button.getAttribute('role') !== 'option' || !button.closest('.model-menu-list'))) return;
    const rows = flat();
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const options = Array.from(root?.querySelectorAll<HTMLButtonElement>('.model-menu-list [role="option"]') ?? []);
      const focused = button ? options.indexOf(button) : -1;
      const start = focused >= 0 ? focused : active();
      const direction = e.key === 'ArrowDown' ? 1 : -1;
      let next = start + direction;
      while (next >= 0 && next < rows.length && locked()) next += direction;
      if (next < 0 || next >= rows.length) return;
      setActive(next);
      // From the search box nothing takes the focus, so the row has to come
      // into view by itself: Enter chooses the highlighted row, seen or not
      // (a Codex read; the menu's height cap puts rows below the list's edge).
      if (button) options[next]?.focus(); else options[next]?.scrollIntoView({ block: 'nearest' });
    }
    else if (e.key === 'Enter') { e.preventDefault(); const row = rows[active()]; if (row) choose(row); }
    else if (e.key === 'Escape') {
      e.preventDefault();
      // Focus goes back to the model button, not to the page (2026-11-01 review).
      const inside = !!root?.contains(document.activeElement);
      setOpen(false);
      if (inside) trigger?.focus();
    }
  }

  const triggerLabel = () => modelLabel(AGENT_LABELS[chatAgent()], chatModel(), catalogs()[chatAgent()]);
  const indexOf = (row: MenuRow) => flat().indexOf(row);

  const Row = (props: { row: MenuRow }) => {
    const checked = () => !props.row.typed && sameRow(props.row, current(), catalogs()[props.row.agent]);
    const effort = () => effortLabel(effortOf(props.row.agent));
    return (
      <button
        type="button"
        role="option"
        class="model-menu-row"
        classList={{ active: indexOf(props.row) === active() }}
        aria-selected={checked()}
        disabled={locked()}
        onMouseEnter={() => setActive(indexOf(props.row))}
        onFocus={() => setActive(indexOf(props.row))}
        onClick={() => choose(props.row)}
      >
        <AgentMark agent={props.row.agent} />
        <span class="model-menu-label">
          <span class="model-menu-name">{props.row.label}</span>
          {/* Cua CU4 and CU8: why this agent can't use the desktop tools yet. */}
          <Show when={!props.row.typed && agentDescriptors().find(d => d.id === props.row.agent)?.capabilities.desktopUnavailable}>{why => (
            <small class="model-menu-note" title={why()}>Can’t use the desktop in this chat</small>
          )}</Show>
        </span>
        <Show when={effort() && !props.row.typed}><span class="model-menu-effort">{effort()}</span></Show>
        <span class="model-menu-check"><Show when={checked()}><Icon name="check" size={14} /></Show></span>
      </button>
    );
  };

  return (
    <div class="model-menu" ref={root}>
      <button
        type="button"
        ref={trigger}
        class="model-menu-trigger"
        disabled={agentsAvailable().length === 0}
        onPointerDown={() => { pressingTrigger = true; }}
        onPointerUp={() => { pressingTrigger = false; }}
        onPointerCancel={() => { pressingTrigger = false; }}
        onClick={() => { pressingTrigger = false; setOpen(o => !o); }}
        aria-label={`Model: ${AGENT_LABELS[chatAgent()]} · ${chatModel() || 'default'} · ${permissionsTriggerLabel(permLabels()[permMode()], permDanger())}`}
        aria-expanded={open()}
        title={locked() ? 'The model and permissions can change once this turn finishes' : `${triggerLabel()}: model, effort and permissions for this chat (${permLabels()[permMode()]})`}
      >
        <AgentMark agent={chatAgent()} size={14} />
        <span class="model-menu-trigger-name">{triggerLabel()}</span>
        <Show when={effortShown(effortOf(chatAgent()), chatUltracode())}>{e => <span class="model-menu-effort">{e()}</span>}</Show>
        <Icon name="chevronDown" size={12} />
      </button>
      <Show when={phase() !== 'closed'}>
        <div class={`model-menu-pop ${phase() === 'closing' ? 'is-closing' : ''}`} style={room() != null ? { '--model-pop-room': `${room()}px` } : undefined} onKeyDown={onKey}
          onFocusOut={e => { if (!pressingTrigger && focusLeavesMenu(e.relatedTarget as Element | null, e.currentTarget, trigger, n => inSelectPopover(n as EventTarget | null))) setOpen(false); }}>
          <label class="model-menu-search">
            <Icon name="search" size={14} />
            <input
              ref={search}
              type="text"
              placeholder="Search models"
              aria-label="Search models"
              spellcheck={false}
              value={query()}
              onInput={e => setQuery(e.currentTarget.value)}
            />
          </label>
          {/* Plan L3 (2026-10-14; review V05, matrix X10): say before the
              switch what it does to the conversation, not only afterwards
              in the "started a new session" notice. */}
          <Show when={chatTurns.length > 0 && chatSessionId()}>
            <p class="model-menu-note" role="note">Another agent or model starts a new agent session. Hollow writes the latest part of this conversation into it (up to 24 entries); everything stays here.</p>
          </Show>
          <Show when={healthError()}><div class="model-menu-note" role="status">Agent status could not refresh. Any status below is the last known value. <button type="button" class="btn small" onClick={() => setHealthRetry(n => n + 1)}>Retry agent status</button></div></Show>
          <div class="model-menu-list" role="group" aria-label="Models">
            <For each={groups()}>
              {g => (
                <div class="model-menu-group" role="group" aria-label={g.agentLabel}>
                  <div class="model-menu-group-label">{g.agentLabel}</div>
                  <div role="listbox" aria-label={`${g.agentLabel} models`}><For each={g.rows}>{row => <Row row={row} />}</For></div>
                  <Suspense><ModelMenuHealth label={g.agentLabel} warning={warnings()[g.agent]} loading={!!loading()[g.agent]}
                    refresh={() => load(g.agent, true)} row={health().find(row => row.agent === g.agent)}
                    support={chatSteerSupport()?.find(row => row.agent === g.agent)} close={() => setOpen(false)} /></Suspense>
                </div>
              )}
            </For>
            <Show when={extras().length > 0}>
              <div class="model-menu-group" role="listbox" aria-label="Custom model ID">
                <div class="model-menu-group-label">Custom model ID</div>
                <For each={extras()}>{row => <Row row={row} />}</For>
              </div>
            </Show>
            <Show when={flat().length === 0}><p class="model-menu-empty">No models match.</p></Show>
          </div>
          <Show when={levels(chatAgent()).length > 0}>
            <div class="model-menu-divider" />
            <button type="button" ref={effortRow} class="model-menu-setting" aria-expanded={effortOpen()} onClick={() => { setCanvasesOpen(false); setPermOpen(false); setEffortOpen(o => !o); }}>
              <span>Effort</span>
              <span class="model-menu-setting-value">{effortShown(effortOf(chatAgent()), chatUltracode()) ?? 'Default'}</span>
              <Icon name={effortOpen() ? 'chevronDown' : 'chevronRight'} size={12} />
            </button>
            <Show when={effortOpen()}>
              <div class="model-menu-efforts" data-setting="effort" role="listbox" aria-label={`${AGENT_LABELS[chatAgent()]} effort`}>
                <For each={['', ...chatLevels()]}>
                  {level => (
                    <button type="button" role="option" class="model-menu-row" aria-selected={!chatUltracode() && (chatEffort() ?? '') === level} onClick={() => void setEffort(level)}>
                      <span class="model-menu-name">{level ? effortLabel(level) : 'Default'}</span>
                      <span class="model-menu-check"><Show when={!chatUltracode() && (chatEffort() ?? '') === level}><Icon name="check" size={14} /></Show></span>
                    </button>
                  )}
                </For>
                <button type="button" role="option" class="model-menu-row" aria-selected={chatUltracode()} disabled={locked()}
                  title={ultracodeNote(chatAgent(), chatUltracode())}
                  onClick={() => { setChatUltracodePersistent(!chatUltracode()); setEffortOpen(false); queueMicrotask(() => effortRow?.focus()); }}>
                  <span class="model-menu-name">Ultracode</span>
                  <span class="model-menu-effort">this chat</span>
                  <span class="model-menu-check"><Show when={chatUltracode()}><Icon name="check" size={14} /></Show></span>
                </button>
              </div>
            </Show>
          </Show>
          <Show when={levels(chatAgent()).length === 0}><div class="model-menu-divider" /></Show>
          <button type="button" ref={permRow} class="model-menu-setting" classList={{ danger: permDanger() }} aria-expanded={permOpen()}
            aria-label={`${permissionsTriggerLabel(permLabels()[permMode()], permDanger())}. Change`}
            onClick={() => { setEffortOpen(false); setCanvasesOpen(false); setPermOpen(o => !o); }}>
            <span>Permissions</span>
            <span class="model-menu-setting-value">{permLabels()[permMode()]}</span>
            <Icon name={permOpen() ? 'chevronDown' : 'chevronRight'} size={12} />
          </button>
          <Show when={permOpen()}>
            <div class="model-menu-perms" role="group" aria-label="Permission settings">
              <div class="model-menu-efforts" data-setting="permissions" role="listbox" aria-label="Permissions">
                <For each={permModeOptions(agentModeOrder(chatAgent()), permLabels(), agentModeDescriptions(chatAgent()), currentAgentCapabilities())}>
                  {option => (
                    <button type="button" role="option" class="model-menu-row" classList={{ danger: agentModeDanger(chatAgent())[option.value] }}
                      aria-selected={permMode() === option.value} disabled={locked() || !!option.disabled} title={option.detail}
                      onClick={() => void setPermission(option.value)}>
                      <span class="model-menu-name">{option.label}</span>
                      <span class="model-menu-check"><Show when={permMode() === option.value}><Icon name="check" size={14} /></Show></span>
                    </button>
                  )}
                </For>
              </div>
              <p class="model-menu-note model-menu-perm-note">{locked() ? 'Permissions can change once this turn finishes. ' : ''}{permModeNote(permLabels()[permMode()], agentModeDescriptions(chatAgent())[permMode()], currentAgentCapabilities(), permMode())}</p>
              <p class="model-menu-note model-menu-perm-note">{hollowCardsLine(chatAgent(), currentAgentCapabilities()?.ownedMcp)}</p>
            </div>
          </Show>
          <button type="button" ref={canvasesRow} class="model-menu-setting" aria-expanded={canvasesOpen()} disabled={chatSessionLoading()} onClick={() => { setEffortOpen(false); setPermOpen(false); setCanvasesOpen(o => !o); }}>
            <span>Canvases</span>
            <span class="model-menu-setting-value">{canvasesMenuValue(chatCanvases(), canvasSetting())}</span>
            <Icon name={canvasesOpen() ? 'chevronDown' : 'chevronRight'} size={12} />
          </button>
          <Show when={canvasesOpen()}>
            <div class="model-menu-efforts" data-setting="canvases" role="listbox" aria-label="Canvases in this chat">
              <For each={[null, ...CANVAS_FREQUENCIES]}>
                {choice => (
                  <button type="button" role="option" class="model-menu-row" aria-selected={chatCanvases() === choice}
                    onClick={() => { setChatCanvasesPersistent(choice); setCanvasesOpen(false); queueMicrotask(() => canvasesRow?.focus()); }}>
                    <span class="model-menu-name">{choice ? CANVAS_FREQUENCY_LABELS[choice] : `Use the setting (${CANVAS_FREQUENCY_LABELS[canvasSetting()]})`}</span>
                    <Show when={choice}><span class="model-menu-effort">this chat</span></Show>
                    <span class="model-menu-check"><Show when={chatCanvases() === choice}><Icon name="check" size={14} /></Show></span>
                  </button>
                )}
              </For>
            </div>
          </Show>
          <div class="model-menu-footer">
            {/* Settings → Chat holds this menu's defaults (Canvases) and how chats start (P1-22).
                The request alone only moved an open Settings; it goes there first. */}
            <button type="button" class="model-menu-edit" onClick={() => { setOpen(false); openSettingsFromOutside('chat'); }}>
              <Icon name="settings" size={13} /> Edit
            </button>
            <span class="model-menu-hint">{chatUltracode() ? ultracodeNote(chatAgent(), true) : `Effort is for this chat only. The ${AGENT_LABELS[chatAgent()]} default is in Settings`}</span>
          </div>
        </div>
      </Show>
    </div>
  );
}
