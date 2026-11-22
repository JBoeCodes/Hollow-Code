import { createEffect, createMemo, lazy, on, onMount, Show } from 'solid-js';
import { OutsideSuspense } from '../components/OutsideSuspense';
import { RecoverableView } from '../components/RecoverableView';
import { preloadWhenIdle } from '../components/OnceOpened';
import { PageHeader, Tabs } from '../components/ui';
import { REVIEW_VIEWS, openReviewView, reviewView, type ReviewView } from '../store/destinations';
import { refreshWaitingOnYou, setWaitingFilter, waitingFilter, waitingTabCounts } from '../store/waitingOnYou';
import '../styles/destinations.css';
import '../styles/review.css';

const PANEL = () => import('../components/WaitingOnYouPanel');
const WaitingOnYouPanel = lazy(() => PANEL().then(m => ({ default: m.default })));

/**
 * Needs you: the full decision view behind the bell's See all.
 * Waiting is one list with All, Decisions, Results and Failures.
 * Incoming was cut (overnight 11-08 S3). Answering an approval and
 * accepting a Result stay separate actions.
 */
export function Review() {
  onMount(() => { refreshWaitingOnYou(true); preloadWhenIdle([PANEL]); });
  createEffect(on(reviewView, () => refreshWaitingOnYou(), { defer: true }));
  const counts = createMemo(waitingTabCounts);
  const count = (v: ReviewView): number | undefined => counts()[v];
  const choose = (v: ReviewView) => { if (v !== 'all' && waitingFilter()) setWaitingFilter(null); openReviewView(v); };
  return <div class="review-page">
    <PageHeader title="Needs you" />
    <div class="review-body">
    <Tabs class="destination-views" label="Waiting filters" items={REVIEW_VIEWS.map(v => ({ value: v.id, label: v.label, count: count(v.id), hint: v.hint }))} value={reviewView()} onChange={choose} />
    <Show when={waitingFilter()}><span class="sr-only" role="status">Filtered to one page's review items</span></Show>
    <OutsideSuspense>{() => <RecoverableView name="Review" compact suspends={false}><WaitingOnYouPanel view={reviewView()} /></RecoverableView>}</OutsideSuspense>
    </div>
  </div>;
}
