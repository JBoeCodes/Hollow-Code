// Desktop finish announcements (0.4): watches every chat's state and, when one finishes or
// needs you, shows one native banner (bursts coalesce) and keeps the dock badge in step.
// Lazy: it renders nothing. The rules are pure in ../threadNotifications.ts.
import { createEffect, createSignal, onCleanup } from 'solid-js';
import { api } from '../api';
import { chatAgentSessionKey, conversationHistoryShown, activePage, notifyNative } from '../store/runtime';
import { sessionStatesNow } from '../store/sessionStates';
import { refreshWorkspaceSessions, workspaceId, workspaceSessions } from '../store/workspaces';
import { windowAway } from '../store/noticeTimers';
import { chatAnnouncements } from '../store/threadAnnounce';
import { badgeCount, bannerFor, chatEvents, shouldAnnounce, type ChatEvent } from '../threadNotifications';
import type { SessionState } from '../sessionIndicator';

/** Events wait this long so a burst (an overnight batch) becomes one banner. */
const COALESCE_MS = 1500;

export default function ThreadNotificationCoordinator() {
  let prev: ReadonlyMap<string, SessionState> | null = null;
  const lastShown = new Map<string, number>();
  const [unseen, setUnseen] = createSignal<ReadonlySet<string>>(new Set());
  let pending: ChatEvent[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let lastBadge = -1;

  const sessions = () => Object.values(workspaceSessions()).flatMap(g => g?.sessions ?? []);
  // A subagent never announces. A key the list has not shown yet is unknown: it waits for the
  // flush (after a re-read of the list) and is dropped there if it is still unknown, so a subagent
  // outside the newest-25 window can't leak a banner. Once seen as a subagent it stays one.
  const subagents = new Set<string>();
  const isSubagent = (key: string) => {
    if (subagents.has(key)) return true;
    if (sessions().some(s => s.key === key && s.forkKind === 'subagent')) { subagents.add(key); return true; }
    return false;
  };
  const isUnknown = (key: string) => !sessions().some(s => s.key === key);
  const isSubagentOrUnknown = (key: string) => isSubagent(key) || isUnknown(key);
  const titleOf = (key: string) => sessions().find(s => s.key === key)?.title ?? '';
  const onScreenKey = () => ((activePage() === 'today' && !!workspaceId()) || conversationHistoryShown() ? chatAgentSessionKey() : null);

  // After the coalescing wait the world may have moved: a chat that came back to running was
  // a flap (or a state map refreshing), one now on screen in a focused window is being watched.
  const stillTrue = (e: ChatEvent, states: ReadonlyMap<string, SessionState>) => {
    const now = states.get(e.key) ?? 'idle';
    return e.kind === 'finished' ? now === 'idle' : e.kind === 'needs-you' ? now === 'waiting' : now === 'failed';
  };
  const flush = () => {
    timer = undefined;
    const states = sessionStatesNow();
    const at = Date.now();
    const events = pending.filter(e => stillTrue(e, states)
      && shouldAnnounce(e, { enabled: chatAnnouncements(), focused: !windowAway(), onScreenKey: onScreenKey(), lastShown, now: at, isSubagent: isSubagentOrUnknown }));
    pending = [];
    const banner = bannerFor(events, titleOf);
    if (!banner || !notifyNative()) return;
    const now = at;
    for (const e of events) lastShown.set(e.key, now);
    void api.announceChatNative(banner.title, banner.body).catch(() => { /* not in the desktop app */ });
  };

  // New events: banner (under the quiet rules) and the unseen-finished set.
  createEffect(() => {
    const next = sessionStatesNow();
    const events = chatEvents(prev, next);
    prev = next;
    if (events.length === 0) return;
    const focused = !windowAway();
    const watching = onScreenKey();
    const ctx = { enabled: chatAnnouncements(), focused, onScreenKey: watching, lastShown, now: Date.now(), isSubagent };
    const shown = events.filter(e => shouldAnnounce(e, ctx));
    if (shown.some(e => isUnknown(e.key))) void refreshWorkspaceSessions();
    if (shown.length) {
      // One entry per chat: the latest event for it wins.
      pending = [...pending.filter(p => !shown.some(e => e.key === p.key)), ...shown];
      if (timer === undefined) timer = setTimeout(flush, COALESCE_MS);
    }
    if (chatAnnouncements()) {
      const fresh = events.filter(e => e.kind === 'finished' && !isSubagentOrUnknown(e.key) && !(focused && watching === e.key));
      if (fresh.length) setUnseen(s => new Set([...s, ...fresh.map(e => e.key)]));
    }
  });

  // Looking at a chat in a focused window clears its unseen mark. So does a chat that runs
  // again or is gone (archived, deleted): a mark for a chat nobody can open would pin the badge.
  createEffect(() => {
    const key = onScreenKey();
    const watched = key && !windowAway() ? key : null;
    const states = sessionStatesNow();
    const known = new Set(sessions().map(x => x.key));
    setUnseen(s => {
      const keep = [...s].filter(k => k !== watched && known.has(k) && (states.get(k) ?? 'idle') === 'idle');
      return keep.length === s.size ? s : new Set(keep);
    });
  });

  // The dock badge follows the count; the switch off clears it.
  createEffect(() => {
    const n = chatAnnouncements() ? badgeCount(sessionStatesNow(), unseen(), isSubagentOrUnknown) : 0;
    if (n === lastBadge) return;
    lastBadge = n;
    void api.setDockBadge(n).catch(() => { lastBadge = -1; /* not in the desktop app: retry on the next change */ });
  });

  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
    if (lastBadge > 0) void api.setDockBadge(0).catch(() => { /* ignore */ });
  });
  return null;
}
