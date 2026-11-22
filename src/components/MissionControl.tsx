import { ViewToastRegion } from './ViewToastRegion';
import { ActivityDock } from './ActivityDock';
// Mission Control (overnight 10-11 B2): every running and queued agent turn
// at once, as an orbit (a star per workspace, a satellite per turn) or a
// list of cards, with Open / Stop / Note, and today's finished turns below.
// A lazy chunk, opened with `g o` or the palette. Live from the run queue
// (store/runQueue.ts) and the feed (store/missionControl.ts); nothing here
// polls the backend except the day's spend, once a minute while open.
import { EmptyArt } from './EmptyArt';
import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal, lazy, on, onCleanup, onMount } from 'solid-js';
import { createStore, reconcile } from 'solid-js/store';
import { api } from '../api';
import { AGENT_LABELS, KNOWN_AGENTS, chatInput, commandPaletteOpen, confirmDialog, openWorkflowBySlug, setChatInput, showConfirm, showPrompt } from '../store';
import { resumePausedChat, windowAway } from '../store/runtime';
import { chatResumingFor, pausedChats } from '../store/chatPause';
import { pauseAllChats, pausedCount, refreshPausedChats, resumeAllChats } from '../store/chatPauseActions';
import { refreshRunQueue, runQueue, runQueueRead } from '../store/runQueue';
import { rowStopNote } from '../runQueue';
import { workflowActivity } from '../store/runActivity';
import { closeMissionControl, missionControlOpen, missionFeed } from '../store/missionControl';
import { goToPage, openSessionByKey, workspaceSessions, workspaces } from '../store/workspaces';
import { addToDraft } from '../chatQueue';
import { Icon } from '../icons';
import { errMsg } from '../utils';
import { canResume, elapsed, missionCards, money, orbitLayout, phaseLine, stopCard, type CardStatus, type ChatPlace, type MissionCard, type OrbitSystem } from '../missionControl';
import { AgentMark } from './AgentMark';
import { createExitPhase } from './exit';
import { Segmented, StatusPill, PlainErrorText, ErrorText } from './ui';
import { latestUnlessErrored } from '../stableResource';
import { QUICK_CHATS_ID, QUICK_CHATS_LABEL } from '../workspaceTypes';
import { keyInView, shortcutsEnabled } from '../shortcuts';
import { SessionIndicator, SessionRollupBadge } from './SessionIndicator';
import { sessionState } from '../store/sessionStates';
import { SESSION_STATE_WORDS, rollupSessions, stateOfRunStatus, type SessionState } from '../sessionIndicator';
import { missionTracks } from '../missionTracks';
import { MissionTracks } from './MissionTracks';
import '../styles/mission-control.css';

type View = 'tracks' | 'orbit' | 'list';
const VIEW_KEY = 'hollow.missionView';
const VIEWS: readonly { value: View; label: string }[] = [{ value: 'tracks', label: 'Tracks' }, { value: 'orbit', label: 'Orbit' }, { value: 'list', label: 'List' }];

const STATUS: Record<CardStatus, { label: string; tone: string }> = {
  running: { label: 'Running', tone: 'busy' },
  starting: { label: 'Preparing', tone: 'busy' },
  queued: { label: 'Queued', tone: 'neutral' },
  done: { label: 'Done', tone: 'ok' },
  failed: { label: 'Failed', tone: 'error' },
  paused: { label: 'Paused', tone: 'warn' },
  stopped: { label: 'Stopped', tone: 'off' },
  skipped: { label: 'Skipped', tone: 'off' },
  interrupted: { label: 'Interrupted', tone: 'warn' },
};
const STOP_BODY: Record<NonNullable<MissionCard['stop']>, string> = {
  Stop: 'The turn stops where it is. What it already changed stays.',
  Skip: 'This scheduled run is skipped this time; the schedule stays.',
  Cancel: 'It leaves the queue and never starts.',
};

const agentName = (agent: string | null) => {
  if (!agent) return null;
  const known = KNOWN_AGENTS.find(a => a === agent);
  return known ? AGENT_LABELS[known] : agent;
};
const savedView = (): View => { try { const v = localStorage.getItem(VIEW_KEY); return v === 'list' || v === 'orbit' ? v : 'tracks'; } catch { return 'tracks'; } };

/** A note can go to a workspace chat: opening one keeps the draft. */
const canNote = (c: MissionCard) => !!c.key && c.system !== 'chats';

/** P1c: a paused chat's "Resume at..." control, loaded once one shows. */
const ResumeAtControl = lazy(() => import('./chat/ResumeAtControl'));

export function MissionControl() {
  const phase = createExitPhase(missionControlOpen, 140);
  const emptyView = () => <div class="mc-empty" role="status"><EmptyArt kind="mission" evidence={{ready:runQueueRead().loaded, failed:!!runQueueRead().error}}/><p>{runQueueRead().error ? 'The run list is unavailable.' : !runQueueRead().loaded ? 'Loading running turns…' : 'Nothing is running. Turns you start, and automations, appear here as they run.'}</p></div>;
  let card: HTMLDivElement | undefined;
  const [view, setViewRaw] = createSignal<View>(savedView());
  const setView = (v: View) => { setViewRaw(v); try { localStorage.setItem(VIEW_KEY, v); } catch { /* memory holds it */ } };
  const [selected, setSelected] = createSignal<string | null>(null);
  const [note, setNote] = createSignal<{ text: string; error: boolean } | null>(null);
  const [now, setNow] = createSignal(Date.now());
  const [spent, { refetch: refetchSpent }] = createResource(() => missionControlOpen() || null, () => api.usageSummary('day'));

  // Where each conversation lives: its workspace, title and agent.
  const places = createMemo(() => {
    // The quick-chats folder belongs to the COO (11-10) and says so.
    const titles = new Map(workspaces().map(w => [w.id, w.id === QUICK_CHATS_ID ? QUICK_CHATS_LABEL : w.title]));
    const out = new Map<string, ChatPlace>();
    for (const [id, group] of Object.entries(workspaceSessions())) {
      for (const s of group.sessions) out.set(s.key, { workspaceId: id, workspaceTitle: titles.get(id) ?? id, title: s.title?.trim() || 'Conversation', agent: s.agent || null });
    }
    return out;
  });
  // Noon of today: changes once a day, not with every tick.
  const today = createMemo(() => { const d = new Date(now()); d.setHours(12, 0, 0, 0); return d.getTime(); });
  // Cards live in a store reconciled by id, so a card, a satellite and their
  // DOM survive every refresh and token (a rebuilt satellite would restart
  // its orbit). Computed only while open: the overlay stays mounted once
  // opened, and the feed changes with every streamed token.
  const [board, setBoard] = createStore<{ live: MissionCard[]; finished: MissionCard[]; systems: OrbitSystem[] }>({ live: [], finished: [], systems: [] });
  createEffect(() => {
    if (!missionControlOpen()) return;
    const next = missionCards(runQueue(), missionFeed(), key => places().get(key) ?? null, today(), workflowActivity(), new Set(Object.keys(pausedChats())));
    setBoard('live', reconcile(next.live, { key: 'id' }));
    setBoard('finished', reconcile(next.finished, { key: 'id' }));
    setBoard('systems', reconcile(orbitLayout(next.live), { key: 'id' }));
  });
  const live = () => board.live;
  const systems = () => board.systems;
  const selectable = createMemo(() => view() === 'tracks' ? missionTracks(live(), board.finished).flatMap(t => t.cards) : live());
  const current = () => selectable().find(c => c.id === selected()) ?? selectable()[0] ?? null;
  const counts = createMemo(() => ({
    running: live().filter(c => c.status === 'running' || c.status === 'starting').length,
    queued: live().filter(c => c.status === 'queued').length,
    finished: board.finished.length,
  }));
  // The ledger sums reported and estimated rows apart; the day is both.
  const spentToday = () => {
    const t = latestUnlessErrored(spent)?.total;
    if (!t || (t.reportedCostUsd === null && t.estimatedCostUsd === null)) return null;
    const total = money((t.reportedCostUsd ?? 0) + (t.estimatedCostUsd ?? 0));
    return t.estimatedCostUsd ? `≈${total}` : total;
  };

  createEffect(on(missionControlOpen, isOpen => {
    if (!isOpen) return;
    setNote(null);
    refreshRunQueue(0);
    // P1b, P1c: every paused chat's note, reason and resume time.
    void refreshPausedChats();
    void refetchSpent();
  }));
  createEffect(on(phase, p => {
    if (p !== 'open') return;
  }));
  // The clock ticks while open and in front; the spend refreshes each minute.
  createEffect(() => {
    if (!missionControlOpen() || windowAway()) return;
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const minute = window.setInterval(() => void refetchSpent(), 60_000);
    onCleanup(() => { window.clearInterval(tick); window.clearInterval(minute); });
  });

  const open = (c: MissionCard) => {
    closeMissionControl();
    if (c.key) void openSessionByKey(c.key);
    // An automation's run opens that automation: its latest result and runs.
    else if (c.slug) openWorkflowBySlug(c.slug);
    else goToPage('workflows');
  };
  const stop = async (c: MissionCard) => {
    const verb = c.stop;
    if (!verb) return;
    // By the card's row, read before the confirm opens (`stopCard`): a stop
    // by key could end the next turn of the same chat.
    const confirm = () => showConfirm({ title: `${verb} “${c.title}”?`, body: STOP_BODY[verb], confirmLabel: verb, danger: true });
    try {
      const done = await stopCard(c, missionFeed, { confirm, cancelRow: api.cancelQueuedRun });
      if (done === 'declined') return;
      setNote({ text: rowStopNote(done, c.title) ?? (verb === 'Stop' ? `Stopping “${c.title}”.` : verb === 'Skip' ? `Skipping “${c.title}” this time.` : `Cancelled “${c.title}”.`), error: false });
    } catch (e) {
      setNote({ text: `Couldn’t stop “${c.title}”: ${errMsg(e)}`, error: true });
    }
    refreshRunQueue(0);
  };
  // P1a PR 2: a paused chat's Resume, the same as the composer's.
  const resume = async (c: MissionCard) => {
    if (!c.key || !canResume(c)) return;
    if (await resumePausedChat(c.key)) setNote({ text: `Resuming “${c.title}”.`, error: false });
    refreshRunQueue(0);
  };
  // P1b: Pause all running chats at their next safe point, and Resume all.
  const runningChats = () => live().filter(c => c.key && (c.status === 'running' || c.status === 'starting')).length;
  const [allBusy, setAllBusy] = createSignal(false);
  const pauseAll = async () => {
    setAllBusy(true);
    try {
      const asked = await pauseAllChats();
      setNote({ text: asked ? `Pausing ${asked} ${asked === 1 ? 'chat' : 'chats'} at the next safe point. A notification lists what each was doing once they have paused.` : 'Nothing to pause: no chat is running.', error: false });
    } catch (e) {
      setNote({ text: `Couldn’t pause all: ${errMsg(e)}`, error: true });
    } finally { setAllBusy(false); }
  };
  const resumeAll = async () => {
    setAllBusy(true);
    try {
      const r = await resumeAllChats();
      const words = r.resumed ? `Resuming ${r.resumed} ${r.resumed === 1 ? 'chat' : 'chats'}.` : 'Nothing resumed.';
      const waiting = r.skipped ? ` ${r.skipped} ${r.skipped === 1 ? 'waits' : 'wait'} for ${r.skipped === 1 ? 'its' : 'their'} own resume time.` : '';
      setNote({ text: (r.refused ? `${words}${waiting} ${r.refused} couldn’t: ${r.firstError ?? ''}` : `${words}${waiting}`).trim(), error: r.refused > 0 && !r.resumed });
    } catch (e) {
      setNote({ text: `Couldn’t resume all: ${errMsg(e)}`, error: true });
    } finally { setAllBusy(false); refreshRunQueue(0); }
  };
  const addNote = async (c: MissionCard) => {
    if (!c.key || !canNote(c)) return;
    const text = await showPrompt({
      title: `Note for “${c.title}”`,
      body: 'Your note goes into this chat’s message box. Send it there: it steers the running turn where its agent can take a message mid-turn, or goes when the turn ends (Send options has the choices).',
      confirmLabel: 'Open with note',
      input: { value: '', label: 'Note', placeholder: 'Also check the Q4 numbers' },
    });
    if (!text) return;
    closeMissionControl();
    if (await openSessionByKey(c.key)) setChatInput(addToDraft(chatInput(), text));
  };

  const move = (step: number) => {
    const list = selectable();
    if (!list.length) return;
    const at = Math.max(0, list.findIndex(c => c.id === current()?.id));
    const next = list[(at + step + list.length) % list.length];
    setSelected(next.id);
    queueMicrotask(() => card?.querySelector<HTMLElement>(`[data-mc-id="${next.id}"]`)?.focus());
  };

  onMount(() => {
    // Capture phase, like the workspace map; a dialog or the palette on top
    // owns the keys.
    const onKey = (e: KeyboardEvent) => {
      if (!missionControlOpen() || confirmDialog() || commandPaletteOpen()) return;
      // The selected Work view button remains focused after opening Running.
      // Let j/k enter its list without moving focus on mount or taking Enter
      // and Tab away from the navigation button.
      const fromViewButton = (e.key === 'j' || e.key === 'k') && e.target ===
        card?.closest('.your-work')?.querySelector('.destination-views button[aria-pressed="true"]');
      if (!keyInView(card, e.target) && !fromViewButton) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const t = e.target as HTMLElement | null;
      if (t?.closest?.('input, textarea, [contenteditable="true"]')) return;
      // Letters go off with Use keyboard shortcuts, as in the other views.
      if (e.key.length === 1 && !shortcutsEnabled()) return;
      const c = current();
      const act = (fn: () => void) => { e.preventDefault(); e.stopPropagation(); fn(); };
      switch (e.key) {
        case 'j': case 'ArrowDown': return act(() => move(1));
        case 'k': case 'ArrowUp': return act(() => move(-1));
        case 'Enter': if (c && (!t?.closest?.('button') || t.closest('.mc-track-segment'))) act(() => open(c)); return;
        case 's': if (c?.stop) act(() => void stop(c)); return;
        case 'n': if (c && canNote(c)) act(() => void addNote(c)); return;
        case 'v': return act(() => setView(VIEWS[(VIEWS.findIndex(v => v.value === view()) + 1) % VIEWS.length].value));
      }
    };
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => document.removeEventListener('keydown', onKey, true));
  });

  return (
    <Show when={phase() !== 'closed'}>
      <div class="mc-view" classList={{ 'is-closing': phase() === 'closing' }}>
        <div class="mc-card" ref={card} tabIndex={-1} role="region" aria-label="Running">
          <header class="mc-head">
            <p class="mc-stats" aria-live="polite">
              <span><b>{counts().running}</b> running</span>
              <span><b>{counts().queued}</b> queued</span>
              <span><b>{counts().finished}</b> finished today</span>
              <span><b>{spentToday() ?? '—'}</b> spent today</span>
            </p>
            <span class="mc-head-actions">
              <Show when={runningChats() > 0}>
                <button type="button" class="btn small" disabled={allBusy()} title="Pause every running chat at its next safe point, keeping each one’s context and a note of what it was doing" onClick={() => void pauseAll()}>Pause all</button>
              </Show>
              <Show when={pausedCount() > 0}>
                <button type="button" class="btn small" disabled={allBusy()} title="Resume every paused chat where it left off" onClick={() => void resumeAll()}>Resume all</button>
              </Show>
            </span>
            <Segmented level="secondary" label="Running layout" items={VIEWS} value={view()} onChange={setView} class="mc-views" />
          </header>
          <ViewToastRegion />
          <Show when={spent.error}>{error => <ErrorText class="mc-note is-error" text={`Couldn't read today's spend: ${errMsg(error())}`}>{' '}<button type="button" class="btn-link" disabled={spent.loading} onClick={() => void refetchSpent()}>Retry spend</button></ErrorText>}</Show>
          <Show when={runQueueRead().error}><p class="mc-note is-error" role="alert">Couldn't refresh the run list. {runQueue().length ? 'Showing the last loaded list. ' : ''}<button type="button" class="btn-link" onClick={() => refreshRunQueue(0)}>Retry run list</button></p></Show>
          <Show when={note()}>{n => <p class="mc-note" classList={{ 'is-error': n().error }} role={n().error ? 'alert' : 'status'}><PlainErrorText text={n().text} plain={n().error} /></p>}</Show>
          <div class="mc-body" classList={{ 'is-list': view() === 'list' }}>
            <ActivityDock />
            <Show when={view() === 'tracks'}>
              <div class="mc-tracks-view">
                <MissionTracks empty={emptyView()} live={board.live} finished={board.finished} feed={missionFeed()} selected={current()?.id ?? null} word={cardWord} onSelect={setSelected} />
                <aside class="mc-detail" aria-label="Selected turn">
                  <Show when={current()}>{c => <TurnCard card={c()} now={now()} selected onSelect={() => undefined} onOpen={() => open(c())} onStop={() => void stop(c())} onResume={() => void resume(c())} onNote={() => void addNote(c())} />}</Show>
                </aside>
              </div>
            </Show>
            <Show when={view() !== 'tracks'}>
            <Show when={live().length} fallback={emptyView()}>
              <Show when={view() === 'orbit'} fallback={
                <div class="mc-grid" role="list" aria-label="Running and queued turns">
                  <For each={live()}>{c => <div role="listitem"><TurnCard card={c} now={now()} selected={current()?.id === c.id} onSelect={() => setSelected(c.id)} onOpen={() => open(c)} onStop={() => void stop(c)} onResume={() => void resume(c)} onNote={() => void addNote(c)} /></div>}</For>
                </div>
              }>
                <div class="mc-orbit-view">
                  <div class="mc-sky" role="group" aria-label="Running and queued turns by workspace">
                    <For each={systems()}>{s => (
                      <div class="mc-system" style={{ left: `${s.x * 100}%`, top: `${s.y * 100}%` }}>
                        <span class="mc-ring is-inner" aria-hidden="true" />
                        <Show when={s.satellites.some(x => x.ring === 2)}><span class="mc-ring is-outer" aria-hidden="true" /></Show>
                        <span class="mc-star" classList={{ 'is-automations': s.id === 'automations' }} aria-hidden="true" />
                        <span class="mc-star-title">{s.title}<SessionRollupBadge rollup={rollupSessions(s.satellites.map(x => ({ title: x.card.title, state: cardState(x.card) })))} /></span>
                        <For each={s.satellites}>{sat => (
                          <span class="mc-orbit" classList={{ 'is-still': sat.period === 0, 'is-outer': sat.ring === 2 }} style={{ '--a': `${sat.angle}deg`, '--period': `${sat.period || 1}s` }}>
                            <button type="button" class="mc-sat" data-mc-id={sat.card.id} classList={{ selected: current()?.id === sat.card.id, queued: sat.card.status === 'queued' }}
                              aria-pressed={current()?.id === sat.card.id} aria-label={`${sat.card.title}, ${cardWord(sat.card).toLowerCase()}, in ${s.title}`}
                              onClick={() => setSelected(sat.card.id)} onDblClick={() => open(sat.card)}>
                              <span class="mc-sat-face"><Show when={sat.card.agent} fallback={<Icon name="zap" size={12} />}>{a => <AgentMark agent={a()} size={14} />}</Show></span>
                            </button>
                          </span>
                        )}</For>
                      </div>
                    )}</For>
                  </div>
                  <aside class="mc-detail" aria-label="Selected turn">
                    <Show when={current()}>{c => <TurnCard card={c()} now={now()} selected onSelect={() => undefined} onOpen={() => open(c())} onStop={() => void stop(c())} onResume={() => void resume(c())} onNote={() => void addNote(c())} />}</Show>
                  </aside>
                </div>
              </Show>
            </Show>
            </Show>
            <Show when={view() !== 'tracks'}>
            <section class="mc-finished" aria-labelledby="mc-finished-title">
              <h3 id="mc-finished-title">Finished today</h3>
              <Show when={board.finished.length} fallback={<p class="mc-quiet">Nothing has finished yet today.</p>}>
                <ul>
                  <For each={board.finished}>{c => (
                    <li class="mc-done-row">
                      <CardState card={c} />
                      <span class="mc-done-title">{c.title}</span>
                      <span class="mc-done-meta">{c.systemTitle}{c.startedAt && c.finishedAt ? ` · ${elapsed(c.finishedAt - c.startedAt)}` : ''}{money(c.costUsd) ? ` · ${money(c.costUsd)}` : ''}</span>
                      <button type="button" class="btn ghost small" onClick={() => open(c)}>Open</button>
                      <Show when={canResume(c)}>
                        <button type="button" class="btn small" disabled={chatResumingFor(c.key)} onClick={() => void resume(c)}>Resume</button>
                        <Suspense><ResumeAtControl sessionKey={c.key!} resumeAt={pausedChats()[c.key!]?.resumeAt} now={now()} onResumeNow={() => void resume(c)} /></Suspense>
                      </Show>
                    </li>
                  )}</For>
                </ul>
              </Show>
            </section>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}

/** A card in the session-indicator family (asks 10-11 A2): a chat turn
 *  paused on a card for you is "waiting on you", whatever its queue row
 *  says; live and failed turns wear the indicator; finished ones a pill. */
function cardState(c: MissionCard): SessionState {
  if (c.status === 'running' && c.key && sessionState(c.key) === 'waiting') return 'waiting';
  return stateOfRunStatus(c.status);
}
function cardWord(c: MissionCard): string {
  const st = cardState(c);
  return st === 'waiting' ? SESSION_STATE_WORDS.waiting : STATUS[c.status].label;
}
function CardState(props: { card: MissionCard }) {
  const st = () => cardState(props.card);
  return <Show when={st() !== 'idle'} fallback={<StatusPill quiet label={STATUS[props.card.status].label} tone={STATUS[props.card.status].tone} />}>
    <span class="mc-state" data-state={st()}><SessionIndicator state={st()} size={12} decorative />{cardWord(props.card)}</span>
  </Show>;
}

/** One turn's card: what, where, who, what it's doing, its last lines. */
function TurnCard(props: { card: MissionCard; now: number; selected: boolean; onSelect: () => void; onOpen: () => void; onStop: () => void; onResume: () => void; onNote: () => void }) {
  const c = () => props.card;
  const who = () => [agentName(c().agent), c().model].filter(Boolean).join(' · ');
  const clock = () => c().startedAt ? (c().finishedAt !== null ? elapsed(c().finishedAt! - c().startedAt!) : ['running', 'starting', 'queued'].includes(c().status) ? elapsed(props.now - c().startedAt!) : null) : null;
  return (
    <article class="mc-turn" classList={{ selected: props.selected, [`is-${c().status}`]: true }} data-mc-id={c().id} tabIndex={-1} onClick={props.onSelect} aria-label={`${c().title}, ${cardWord(c()).toLowerCase()}`}>
      <header class="mc-turn-head">
        <span class="mc-turn-mark" aria-hidden="true"><Show when={c().agent} fallback={<Icon name="zap" size={14} />}>{a => <AgentMark agent={a()} size={16} />}</Show></span>
        <span class="mc-turn-what"><strong>{c().title}</strong><small>{c().systemTitle}{who() ? ` · ${who()}` : ''}</small></span>
        <CardState card={c()} />
      </header>
      <Show when={c().waiting || phaseLine(c(), props.now)}>{line => <p class="mc-turn-phase">{line()}</p>}</Show>
      <Show when={c().tool}>{tool => <p class="mc-turn-tool"><Icon name="zap" size={12} /><span>{tool()}</span></p>}</Show>
      <Show when={c().lines.length} fallback={<p class="mc-turn-quiet">{c().status === 'queued' ? 'Not started yet.' : 'No output yet.'}</p>}>
        <ol class="mc-turn-lines" aria-label="Latest output">
          <For each={c().lines}>{l => <li>{l}</li>}</For>
        </ol>
      </Show>
      <Show when={c().error}>{err => <p class="mc-turn-error" role="note">{err()}</p>}</Show>
      <footer class="mc-turn-foot">
        <span class="mc-turn-clock">{clock() ?? '—'}</span>
        <span class="mc-turn-cost">{money(c().costUsd) ?? (c().status === 'running' ? 'cost at the end' : '')}</span>
        <span class="mc-turn-actions">
          <button type="button" class="btn small" onClick={e => { e.stopPropagation(); props.onOpen(); }}>Open</button>
          <Show when={c().stop}>{verb => <button type="button" class="btn small" onClick={e => { e.stopPropagation(); props.onStop(); }}>{verb()}</button>}</Show>
          <Show when={canResume(c())}>
            <button type="button" class="btn small" disabled={chatResumingFor(c().key)} onClick={e => { e.stopPropagation(); props.onResume(); }}>Resume</button>
            <Suspense><ResumeAtControl sessionKey={c().key!} resumeAt={pausedChats()[c().key!]?.resumeAt} now={props.now} onResumeNow={props.onResume} /></Suspense>
          </Show>
          <Show when={canNote(c())}><button type="button" class="btn small ghost" onClick={e => { e.stopPropagation(); props.onNote(); }}>Note</button></Show>
        </span>
      </footer>
    </article>
  );
}
