// M2 leftovers (2026-11-15): another Mac's chats in Library, Conversations.
// Two uses: the "On <Mac>" choice (`machineId` set: every chat of that Mac,
// its state and what it offers, Retry or Cancel) and a project's list (no
// `machineId`: each paired Mac's chats in that project, under the Mac's
// name). A row opens the chat read live from its home (RemoteChatView over
// the page, as from the machine strip); right-click is the remote row menu.
// Rows live in memory only (`store/peerSessions.ts`). Lazy, so the shell and
// Conversations' own chunk pay one import line.
import { For, Show, createMemo, onCleanup, onMount } from 'solid-js';
import type { MachineRef, PeerSessionRow } from '../generated/ipc';
import { followPeerSessions, peerErrors, peerMachines, peerRowsHeld } from '../store/peerSessions';
import { openRemoteChat } from '../store/peerChat';
import { peerNow } from '../store/peerUi';
import { fmtTime } from '../utils';
import { SessionIndicator } from './SessionIndicator';
import { MachineBadge } from './MachineBadge';
import { MachineActions } from './MachineActions';
import { libraryRows, popoverInfo } from './machineStrip';
import { openRemoteRowMenu } from './remoteRowMenu';
import '../styles/peer-machines.css';

const title = (r: PeerSessionRow) => r.title?.trim() || 'New chat';

export default function RemoteConversations(props: { machineId?: string | null; workspaceId: string | null; query: string }) {
  // Conversations may be open while the strip isn't following (it follows
  // only once a Mac is paired, after first paint): follow here too.
  onMount(() => { const stop = followPeerSessions(); onCleanup(stop); });
  const macs = createMemo(() => {
    const here = peerMachines()?.thisMac?.machineId;
    return (peerMachines()?.machines ?? []).filter(m => m.machineId !== here && (!props.machineId || m.machineId === props.machineId));
  });
  return (
    <For each={macs()}>
      {m => {
        const ref = (): MachineRef => ({ machineId: m.machineId, slug: m.slug, name: m.name });
        const held = () => peerRowsHeld().find(h => h.machine.machineId === m.machineId);
        const shown = () => libraryRows(held(), peerNow(), { workspaceId: props.workspaceId, query: props.query });
        const info = () => popoverInfo(m, peerNow(), peerErrors()[m.machineId] ?? null);
        const menu = (e: MouseEvent, r: PeerSessionRow) => openRemoteRowMenu(e, ref(), r, shown().stale, () => void openRemoteChat(m.machineId, r.key));
        // In a project's list a Mac with nothing there says nothing.
        return (
          <Show when={props.machineId || shown().rows.length > 0}>
            <section class="remote-conversations" aria-label={`Chats on ${m.name}`}>
              <div class="rail-group-label chat-session-group-label">On {m.name}</div>
              <Show when={props.machineId && (info().stale || info().detail)}>
                <div class="remote-conversations-status" role="status">
                  <span>{info().status}<Show when={info().detail}>{d => <> {d()}</>}</Show></span>
                  <MachineActions machineId={m.machineId} info={info()} />
                </div>
              </Show>
              <Show when={shown().rows.length > 0} fallback={
                <div class="chat-history-empty"><strong>{props.query.trim() ? 'No conversations found' : `No chats on ${m.name} yet`}</strong></div>
              }>
                <For each={shown().rows}>
                  {r => (
                    <div class="session-row" classList={{ 'is-stale': shown().stale }} onContextMenu={e => menu(e, r)}>
                      <button type="button" class="chat-session-open" aria-label={`Open ${title(r)} on ${m.name}`}
                        title={shown().stale ? `Read-only while ${m.name} is unreachable` : r.preview ?? undefined}
                        onClick={() => void openRemoteChat(m.machineId, r.key)}>
                        <span class="title"><SessionIndicator state={r.needsAnswer ? 'waiting' : r.state === 'running' ? 'running' : 'idle'} size={8} decorative /> {title(r)}</span>
                        <span class="meta"><MachineBadge machine={r.home} needsAnswer={r.needsAnswer} moved={r.movedFrom ? 'from' : undefined} /><span>{fmtTime(r.lastActivityAt)}</span></span>
                      </button>
                    </div>
                  )}
                </For>
              </Show>
            </section>
          </Show>
        );
      }}
    </For>
  );
}
