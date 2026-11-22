// Mission Control (overnight 10-11 B2): whether the overlay is open, and the
// live feed of what each conversation's turn last said and did. The feed
// listens from launch, so a turn started before the overlay opened still has
// its lines; it keeps a few lines for at most FEED_KEYS conversations. A tiny
// module: the shell imports it to mount the lazy view (App.tsx).
import { createSignal } from 'solid-js';
// Retain exact-run activity from launch, even before Running is opened.
import './runActivity';
import { closeWorkView, openWorkView, workViewShown } from './destinations';
import { listen } from '../nativeEvents';
import { applyFeedEvent, workflowFeedEvent, type Feed, type FeedEvent, type WorkflowFeedInput } from '../missionFeed';
import { MOCK_BUILD, isBrowserMockMode } from '../mockMode';

// Since consolidation D (2026-10-13) Mission Control is Your work → Activity.
export const missionControlOpen = () => workViewShown('activity');
export const openMissionControl = () => openWorkView('activity');
export const closeMissionControl = () => closeWorkView('activity');

export const [missionFeed, setMissionFeed] = createSignal<Feed>({});

/** Fold one `chat-event` payload into the feed. */
export function feedChatEvent(evt: FeedEvent, now = Date.now()) {
  setMissionFeed(feed => applyFeedEvent(feed, evt, now));
}

listen<FeedEvent>('chat-event', e => feedChatEvent(e.payload)).catch(() => { /* not in tauri */ });
// Automations stream too (overnight 10-11/12 A7): their lines and tools, keyed by slug.
listen<WorkflowFeedInput>('workflow-event', e => { const evt = workflowFeedEvent(e.payload); if (evt) feedChatEvent(evt); }).catch(() => { /* not in tauri */ });

// The browser mock has no event stream: `?mock=1&missions=1` seeds a feed
// (mockDemo.ts) so the view can be seen and screenshot.
if (MOCK_BUILD && isBrowserMockMode()) {
  try {
    const seed = localStorage.getItem('mockMissionFeed');
    if (seed) setMissionFeed(JSON.parse(seed) as Feed);
  } catch { /* no seed */ }
}
