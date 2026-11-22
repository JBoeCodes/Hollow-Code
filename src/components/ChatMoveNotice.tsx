// M5 UI: the open chat's move, above its composer (`docs/reference/peers.md`,
// "Moving a chat"). A chat moving to another Mac, or waiting for that Mac
// after Keep it here, says so with Keep it here and Release here (behind a
// confirm that says the other Mac may keep a copy). A chat that moved says
// where it lives now with Open there, or, when that Mac was forgotten, that
// it runs on a Mac you forgot. A chat that moved here says from where. Lazy:
// ChatBody always mounts it, and it shows nothing for a chat that never moved.
// M5 offline UI: a moved chat offers Continue here offline (or Open offline
// copy) and says how its last offline copy synced; an offline copy carries
// the offline badge and says whether it waits, synced as its own chat, or
// was kept here and why (chatOfflineWords.ts).
import { Show, createEffect, createSignal, on } from 'solid-js';
import { chatAgentSessionKey } from '../store/runtime';
import { chatHomes, homesWorthReading } from '../store/moveFlags';
import { keepHere, loadChatHomes, releaseHere } from '../store/chatMoves';
import { peerMachines } from '../store/peerSessions';
import { openRemoteChat } from '../store/peerChat';
import { homeNotice, macName } from '../store/chatMoveWords';
import { workspaceSessions } from '../store/workspaces';
import { isOfflineCopy, offlineRows } from '../store/offlineFlags';
import { continueOffline, loadOfflineRows, openHere } from '../store/chatOffline';
import { OFFLINE_BADGE, offlineBadgeLabel, offlineNotice } from '../store/chatOfflineWords';
import '../styles/peer-machines.css';

export default function ChatMoveNotice() {
  // The chat whose Keep it here or Release here is running (per chat: the
  // notice stays mounted while another chat opens).
  const [busy, setBusy] = createSignal<string | null>(null);
  const home = () => { const key = chatAgentSessionKey(); return key ? chatHomes()[key] : undefined; };
  const session = (key: string) => Object.values(workspaceSessions()).flatMap(g => g?.sessions ?? []).find(s => s.key === key);
  const peer = () => home()?.peer ?? offlineRows().find(r => r.offlineKey === chatAgentSessionKey())?.peer ?? null;
  const name = () => { const p = peer(); return p ? macName(peerMachines(), p) : null; };
  const offline = () => { const key = chatAgentSessionKey(); return key ? offlineNotice(key, home(), offlineRows(), name(), session(key)) : null; };
  const notice = () => {
    const h = homeNotice(home(), name());
    const o = offline();
    if (!o) return h;
    return { kind: h?.kind ?? o.kind, keep: !!h?.keep, open: !!h?.open, text: o.text ?? h?.text ?? '' };
  };
  const title = (key: string) => session(key)?.title?.trim() || 'this chat';
  // Read again when another chat opens: a move may have settled since.
  createEffect(on(chatAgentSessionKey, key => {
    if (!key) return;
    // The homes read reads the offline records too once a copy is known.
    if (homesWorthReading((peerMachines()?.machines.length ?? 0) > 0)) void loadChatHomes();
    // An offline copy opened with no homes read: its record may have changed (synced, kept).
    else if (isOfflineCopy(key) || session(key)?.forkKind === 'offline') void loadOfflineRows();
  }));
  const run = async (act: (key: string) => Promise<void>) => {
    const key = chatAgentSessionKey();
    if (!key || busy() === key) return;
    setBusy(key);
    try { await act(key); } finally { setBusy(b => (b === key ? null : b)); }
  };
  const working = () => busy() != null && busy() === chatAgentSessionKey();
  return (
    <Show when={notice()}>{n => (
      <div class="chat-move-notice" data-kind={n().kind} role="status" aria-label="Where this chat runs">
        <Show when={offline()?.badge}>
          <span class="chat-offline-badge" role="img" aria-label={offlineBadgeLabel(offline()?.kind === 'forked' || offline()?.kind === 'fork-here')}>{OFFLINE_BADGE}</span>
        </Show>
        <p class="chat-move-notice-text">{n().text}</p>
        <Show when={n().keep}>
          <button type="button" class="btn sm" disabled={working()} onClick={() => void run(keepHere)}>Keep it here</button>
          <button type="button" class="btn ghost sm" disabled={working()} onClick={() => void run(key => releaseHere(key, title(key)))}>Release here…</button>
        </Show>
        <Show when={n().open}>
          <button type="button" class="btn sm" onClick={() => { const h = home(); if (h) void openRemoteChat(h.peer, h.key); }}>Open on {name()}</button>
        </Show>
        <Show when={offline()?.canContinue}>
          <button type="button" class="btn ghost sm" disabled={working()} onClick={() => void run(key => continueOffline(key, title(key), name()))}>Continue here offline…</button>
        </Show>
        <Show when={offline()?.openCopy}>{copy => (
          <button type="button" class="btn ghost sm" onClick={() => void openHere(copy())}>Open offline copy</button>
        )}</Show>
        <Show when={offline()?.openOriginal}>{original => (
          <button type="button" class="btn ghost sm" onClick={() => void openHere(original())}>Open the original</button>
        )}</Show>
      </div>
    )}</Show>
  );
}
