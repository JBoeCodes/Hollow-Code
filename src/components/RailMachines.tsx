// M2 PR 4: the machine strip (`docs/reference/peers.md`, "The UI"). In the
// sidebar above Shared tools, shown only once another Mac is paired: one row
// per Mac, This Mac first, each with a status dot, its name, a short state and
// a count of chats running there. A Mac's row opens its popover (its newest
// chats, Retry, ssh's sentence); a Mac that needs pairing again opens
// Settings, Devices, Macs with Pair a Mac filled. A chat picked there opens in
// the page pane as RemoteChatView, over the page, until it is closed or the
// person goes elsewhere. Lazy, so the shell pays only Rail's import line.
import { For, Show, Suspense, createEffect, createMemo, createSignal, lazy, on, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import type { PeerSessionRow } from '../generated/ipc';
import { activePage, conversationHistoryShown, openSettingsFromOutside, setActivePage, sidebarCollapsed } from '../store';
import { workspaceId, workspaceSessions } from '../store/workspaces';
import { chatAgentSessionKey } from '../store/chatRuntime';
import { sessionState } from '../store/sessionStates';
import { followPeerSessions, loadPeerMachines, peerErrors, peerMachines, peerRowsHeld } from '../store/peerSessions';
import { closeRemoteChat, openRemoteChat, remoteChat } from '../store/peerChat';
import { askLibraryMachine, askToPair, peerNow, takeLibraryMachineRequest } from '../store/peerUi';
import type { SessionState } from '../sessionIndicator';
import { Icon } from '../icons';
import { Button } from './ui';
import { MachineActions } from './MachineActions';
import { SessionIndicator } from './SessionIndicator';
import { MachineBadge, MachineGlyph } from './MachineBadge';
import { chatHomeGone, machineRows, popoverInfo, stripRows, type StripRow } from './machineStrip';
import { openRemoteRowMenu } from './remoteRowMenu';
import '../styles/peer-machines.css';

const RemoteChatView = lazy(() => import('./RemoteChatView'));

/** Chats the popover lists; the rest are in each project's list. */
const POPOVER_ROWS = 8;

const rowState = (r: PeerSessionRow): SessionState =>
  r.needsAnswer ? 'waiting' : r.state === 'running' ? 'running' : r.state === 'queued' ? 'queued' : 'idle';

export default function RailMachines() {
  const [open, setOpen] = createSignal<{ machineId: string; top: number; left: number } | null>(null);
  // Launch reads only the record of paired Macs (`peer_machines` without
  // `check`: a local file, no network). Only once a Mac is paired does the
  // strip follow them, after first paint: that opens their tunnels and lists.
  let stopFollowing: (() => void) | null = null;
  let pending: number | null = null;
  const unfollow = () => { if (pending != null) { window.clearTimeout(pending); pending = null; } stopFollowing?.(); stopFollowing = null; };
  // One retry if that read fails (a slow start); after that, Settings or a
  // pairing reads it again.
  let retry: number | undefined;
  onMount(() => { void loadPeerMachines(false).then(view => { if (!view) retry = window.setTimeout(() => void loadPeerMachines(false), 3000); }); });
  onCleanup(() => window.clearTimeout(retry));
  createEffect(() => {
    const paired = (peerMachines()?.machines.length ?? 0) > 0;
    if (!paired) { unfollow(); return; }
    if (stopFollowing || pending != null) return;
    pending = window.setTimeout(() => { pending = null; stopFollowing = followPeerSessions(); }, 1200);
  });
  onCleanup(() => { unfollow(); closeRemoteChat(); });
  const clock = peerNow;

  // This Mac's own running chats, for its count chip.
  const localRunning = createMemo(() => Object.values(workspaceSessions())
    .flatMap(g => g?.sessions ?? []).filter(s => sessionState(s.key) === 'running').length);
  const rows = createMemo(() => stripRows(peerMachines(), peerRowsHeld(), localRunning(), clock()));
  const heldOf = (id: string) => peerRowsHeld().find(h => h.machine.machineId === id);
  const machineOf = (id: string) => peerMachines()?.machines.find(m => m.machineId === id) ?? null;

  // A remote chat gives way when the person goes somewhere else.
  createEffect(on(() => [activePage(), workspaceId(), chatAgentSessionKey()], () => { setOpen(null); if (remoteChat()) closeRemoteChat(); }, { defer: true }));
  // ...and when its Mac is forgotten (here or by a Revoke there).
  createEffect(() => { if (chatHomeGone(peerMachines(), remoteChat())) closeRemoteChat(); });

  function click(r: StripRow, el: HTMLElement) {
    if (r.click === 'none') return;
    if (r.click === 'pair') {
      const m = machineOf(r.machineId);
      askToPair(m?.sshHosts[0] ?? peerMachines()?.suggestedAlias ?? '');
      setOpen(null);
      openSettingsFromOutside('devices');
      return;
    }
    if (open()?.machineId === r.machineId) { setOpen(null); return; }
    const rect = el.getBoundingClientRect();
    setOpen({ machineId: r.machineId, top: rect.top, left: rect.right + 8 });
  }

  return (<>
    <Show when={rows().length > 0}>
      <div class="rail-machines" classList={{ 'is-collapsed': sidebarCollapsed() }} role="group" aria-label="Macs">
        <For each={rows()}>
          {r => {
            // The count chip says how many run; the words then just say Online.
            const body = () => <>
              <span class="machine-dot" data-dot={r.dot} aria-hidden="true" />
              <Show when={sidebarCollapsed()} fallback={<>
                <span class="rail-machine-name">{r.name}</span>
                <span class="rail-machine-state">{r.state === 'online' && r.running > 0 ? 'Online' : r.text}</span>
                <Show when={r.running > 0}><span class="rail-machine-count" aria-hidden="true">{r.running}</span></Show>
              </>}><MachineGlyph size={15} /></Show>
            </>;
            return (
              <Show when={r.click !== 'none'} fallback={<div class="rail-machine-row is-this-mac" role="note" title={r.title} aria-label={r.label}>{body()}</div>}>
                <button data-key-item type="button" class="rail-machine-row" data-machine-id={r.machineId} classList={{ 'is-open': open()?.machineId === r.machineId }}
                  title={r.title} aria-label={r.label} aria-haspopup={r.click === 'popover' ? 'dialog' : undefined}
                  aria-expanded={r.click === 'popover' ? open()?.machineId === r.machineId : undefined}
                  onClick={e => click(r, e.currentTarget)}>
                  {body()}
                </button>
              </Show>
            );
          }}
        </For>
      </div>
      <Show when={open()}>
        {o => <MachinePopover machineId={o().machineId} top={o().top} left={o().left} now={clock()} onClose={() => setOpen(null)}
          held={heldOf(o().machineId)} />}
      </Show>
    </Show>
    {/* Outside the strip's Show: the chat outlives no Mac (closed above), and
        never the strip either. */}
    <Show when={remoteChat() ? document.querySelector<HTMLElement>('.canvas .page') : null}>
      {/* Over the page pane, as a chat opens there; with no page pane (it
          never happens in the app) there is no layer rather than one over
          everything. */}
      {page => <Portal mount={page()}><RemoteLayer /></Portal>}
    </Show>
  </>);
}

/** The layer over the page: what it covers is inert (no focus, no clicks,
 *  hidden from screen readers) until the chat closes, including what the page
 *  puts there meanwhile. On close, focus goes back to the Mac's strip row. */
function RemoteLayer() {
  let el: HTMLDivElement | undefined;
  const machineId = remoteChat()?.machineId ?? null;
  onMount(() => {
    const wrapper = el?.parentElement;
    const page = wrapper?.parentElement;
    if (!wrapper || !page) return;
    const covered = new Set<HTMLElement>();
    const cover = () => {
      for (const c of page.children) {
        if (c === wrapper || !(c instanceof HTMLElement) || c.inert) continue;
        c.inert = true;
        covered.add(c);
      }
    };
    cover();
    const watch = new MutationObserver(cover);
    watch.observe(page, { childList: true });
    onCleanup(() => {
      watch.disconnect();
      for (const c of covered) c.inert = false;
      const at = document.activeElement;
      if (machineId && (!at || at === document.body || el?.contains(at))) {
        document.querySelector<HTMLElement>(`.rail-machine-row[data-machine-id="${CSS.escape(machineId)}"]`)?.focus();
      }
    });
  });
  return (
    <div ref={x => (el = x)} class="remote-chat-layer" tabindex="-1">
      <Suspense fallback={<div class="remote-chat-loading" role="status">Opening the chat…</div>}><RemoteChatView /></Suspense>
    </div>
  );
}

function MachinePopover(props: {
  machineId: string; top: number; left: number; now: number; onClose: () => void;
  held: ReturnType<typeof peerRowsHeld>[number] | undefined;
}) {
  const machine = () => peerMachines()?.machines.find(m => m.machineId === props.machineId) ?? null;
  const shown = () => machineRows(props.held, props.now);
  const info = () => { const m = machine(); return m ? popoverInfo(m, props.now, peerErrors()[props.machineId] ?? null) : null; };
  let root: HTMLDivElement | undefined;

  onMount(() => {
    const away = (e: PointerEvent) => {
      const t = e.target as Element | null;
      // The sidebar itself (its rows, its Close in a small window) leaves it
      // open; going elsewhere closes it (RailMachines' effect).
      if (root && t && !root.contains(t) && !t.closest('.work-rail') && !t.closest('.ctx-menu')) props.onClose();
    };
    // A context menu open over it takes Escape first.
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape' && !e.defaultPrevented && !document.querySelector('.ctx-menu')) props.onClose(); };
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('keydown', key);
    root?.querySelector<HTMLElement>('button')?.focus();
    onCleanup(() => { document.removeEventListener('pointerdown', away, true); document.removeEventListener('keydown', key); });
  });
  // The Mac was forgotten meanwhile.
  createEffect(() => { if (peerMachines() && !machine()) props.onClose(); });

  /** Every chat of this Mac, in Library, Conversations under "On <Mac>"
   *  (the popover lists the newest few). */
  function openAll() {
    const id = props.machineId;
    props.onClose();
    askLibraryMachine(id);
    // A refused navigation (a send being accepted) drops the request.
    void Promise.resolve(setActivePage('chat')).then(() => { if (!conversationHistoryShown()) takeLibraryMachineRequest(); });
  }

  function openChat(r: PeerSessionRow) {
    // Read before closing: the popover's props go with it.
    const id = props.machineId;
    props.onClose();
    void openRemoteChat(id, r.key);
  }

  function menu(e: MouseEvent, r: PeerSessionRow) {
    const m = machine();
    if (m) openRemoteRowMenu(e, { machineId: m.machineId, slug: m.slug, name: m.name }, r, shown().stale, () => openChat(r));
  }

  const style = () => {
    const maxTop = Math.max(8, window.innerHeight - 380);
    return { top: `${Math.min(Math.max(8, props.top - 8), maxTop)}px`, left: `${Math.min(props.left, window.innerWidth - 320)}px` };
  };

  return (
    <Show when={machine() && info()}>
      <Portal>
        <div ref={el => (root = el)} class="machine-popover" role="dialog" aria-label={`Chats on ${machine()!.name}`} style={style()}>
          <header class="machine-popover-head">
            <MachineGlyph size={14} />
            <strong>{info()!.heading}</strong>
            <button type="button" class="icon-btn" aria-label="Close" title="Close (Esc)" onClick={() => props.onClose()}><Icon name="x" size={13} /></button>
          </header>
          <p class="machine-popover-status">{info()!.status}</p>
          <Show when={info()!.detail}>{d => <p class="machine-popover-detail">{d()}</p>}</Show>
          <Show
            when={shown().rows.length > 0}
            fallback={<p class="machine-popover-empty">{info()!.stale ? 'No chats from it this session.' : `No chats on ${machine()!.name} yet.`}</p>}
          >
            <ul class="machine-popover-chats" classList={{ 'is-stale': shown().stale }} aria-label={`Chats on ${machine()!.name}`}>
              <For each={shown().rows.slice(0, POPOVER_ROWS)}>
                {r => (
                  <li>
                    <button type="button" class="machine-popover-chat" title={shown().stale ? `Read-only while ${machine()!.name} is unreachable` : r.preview ?? undefined}
                      onClick={() => openChat(r)} onContextMenu={e => menu(e, r)}>
                      <SessionIndicator state={rowState(r)} size={8} decorative />
                      <span class="machine-popover-chat-title">{r.title?.trim() || 'New chat'}</span>
                      <MachineBadge machine={r.home} needsAnswer={r.needsAnswer} moved={r.movedFrom ? 'from' : undefined} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
          <Show when={info()!.retry || info()!.cancel || shown().rows.length > 0}>
            <div class="machine-popover-actions">
              <MachineActions machineId={props.machineId} info={info()!} />
              <Show when={shown().rows.length > 0}>
                <Button variant="ghost" size="sm" icon="message" onClick={openAll}>Open all in Library</Button>
              </Show>
            </div>
          </Show>
        </div>
      </Portal>
    </Show>
  );
}
