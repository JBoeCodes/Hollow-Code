import { For, Show, createMemo, createSignal } from 'solid-js';
import { openWaitingItem, openWaitingOnYou, refreshWaitingOnYou, waitingBadgeState, waitingDrafts, waitingList } from '../store/waitingOnYou';
import { WAITING_GROUPS, draftsNote, unavailableNotice, waitingDisabledReason } from '../waitingOnYou';
import { NotificationHistory } from './RecentActivity';
import { opensInLearnings } from '../learningBulk';
import { Tabs } from './ui';

/** Needs you's summary, or the one notification history (U3b): its saved
 *  filters, folded repeats, listed-only actions and App messages. Only listed
 *  ids can be cleared; unread failures remain in Needs you. */
export default function NotificationCenter(props: { initialNeedsYou: boolean; onClose: () => void }) {
  const [view, setView] = createSignal(props.initialNeedsYou ? 'needs' : 'notifications');
  const tabs = [{ value: 'needs', label: 'Needs you', get count() { return waitingBadgeState().count ?? undefined; } }, { value: 'notifications', label: 'Notifications' }];
  return <>
    <Tabs label="Notification views" items={tabs} value={view()} onChange={setView}/>
    <Show when={view() === 'needs'} fallback={<NotificationHistory onClose={props.onClose}/>}><NeedsYouSummary onClose={props.onClose}/></Show>
    <footer class="notification-center-footer"><button type="button" class="btn small ghost" aria-label="See all needs you" onClick={() => { props.onClose(); openWaitingOnYou(null, 'all'); }}>{view() === 'needs' ? 'See all' : 'See all needs you'}</button></footer>
  </>;
}

/** Navigation only. The full waiting view still owns every human decision;
 *  a pending learning's row opens Library → Learnings, its one home. */
function NeedsYouSummary(props: { onClose: () => void }) {
  const rows = createMemo(() => waitingList.items().filter(item => item.kind !== 'question' || item.state === 'pending').slice(0, 8));
  return <>
    <div class="notification-center-actions"><button type="button" class="btn small ghost" disabled={waitingList.reloading()} onClick={() => refreshWaitingOnYou(true)}>Refresh needs you</button>
      <Show when={waitingDrafts().length}><span class="hint">{draftsNote(waitingDrafts().length)} in the full list</span></Show>
    </div>
    <div class="notification-center-scroll" role="region" aria-label="Needs you summary">
      <Show when={waitingList.failed()}><p role="alert">Couldn’t load what is waiting. {waitingList.loaded() ? 'Showing the last list.' : ''}</p></Show>
      <Show when={!waitingList.loaded() && !waitingList.failed()}><p role="status">Loading what needs you…</p></Show>
      <Show when={unavailableNotice(waitingList.unavailable())}>{notice => <p role="status">{notice()}</p>}</Show>
      <Show when={waitingList.loaded() && !waitingList.failed() && !waitingList.unavailable().length && waitingBadgeState().count === 0}><p class="hint">No decisions waiting.{waitingDrafts().length ? ' Unsaved drafts are in See all.' : ''}</p></Show>
      <ul class="notification-history notification-needs-list"><For each={rows().map(item => item.id)}>{id => {
        const item = () => rows().find(row => row.id === id)!;
        const reason = () => waitingDisabledReason({ kind: 'item', id, item: item() });
        return <li><button type="button" class="notification-needs-item" disabled={!!reason()} title={reason() ?? undefined} onClick={() => { const selected = item(); props.onClose(); if (opensInLearnings(selected)) openWaitingOnYou('learning'); else void openWaitingItem(selected); }}>
          <small>{WAITING_GROUPS.find(group => group.kinds.includes(item().kind))?.label}</small><strong>{item().title}</strong>
          <Show when={reason() || item().detail}><span>{reason() || item().detail}</span></Show>
        </button></li>;
      }}</For></ul>
      <Show when={waitingList.next() || waitingList.items().length > rows().length}><p class="hint">Showing {rows().length} items. See all for the full list.</p></Show>
    </div>
  </>;
}
