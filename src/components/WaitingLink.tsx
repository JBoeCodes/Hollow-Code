import { Show, Suspense, onMount } from 'solid-js';
import { Icon } from '../icons';
import { enableApprovalsResource, reloadReviewQueue, reviewQueueFailed, reviewQueueItems } from '../store';
import { openWaitingOnYou } from '../store/waitingOnYou';
import { preserveDuringRefetch } from '../stableResource';
import { SUGGESTION_CONTEXTS, suggestionContext, type SuggestionContext } from '../workspaceContent';

/**
 * Where a page's review inbox used to be (Tasks, Settings, Memory, the
 * three Automations panels): one compact line that says how many of the
 * page's review items wait and opens the "waiting on you" surface filtered
 * to them (decision 7). It lists nothing itself, so there is no second list.
 * Mounting re-reads the review queue (the old inboxes read it on every
 * mount), and a failed read shows as an unknown count, never as "0 waiting"
 * or a hidden link.
 */
export function WaitingLink(props: { context: SuggestionContext; hideWhenEmpty?: boolean }) {
  enableApprovalsResource();
  onMount(() => void reloadReviewQueue());
  const items = preserveDuringRefetch(reviewQueueItems);
  const title = () => SUGGESTION_CONTEXTS[props.context].title;
  // A plain function, read only under the Suspense below: a memo here would
  // read the queue's first load outside it and suspend the whole page.
  const count = () => (items() ?? [])
    .filter(item => (item.status === 'pending' || item.status === 'applying') && suggestionContext(item) === props.context).length;
  const known = () => !reviewQueueFailed();
  const countText = () => (known() ? `${count()} waiting` : 'count unavailable');
  return (
    <Suspense fallback={null}>
      <Show when={!props.hideWhenEmpty || !known() || count() > 0}>
        <button
          type="button"
          class="waiting-link"
          data-suggestion-context={props.context}
          onClick={() => openWaitingOnYou(props.context)}
          aria-label={`${title()}: ${countText()}. Open them in ${props.context === 'learning' ? 'Library Learnings' : 'Needs you'}`}
        >
          <Icon name="checkSquare" size={13} />
          <span class="waiting-link-title">{title()}</span>
          <span class="waiting-link-count">{countText()}</span>
          <span class="waiting-link-go">Open in {props.context === 'learning' ? 'Learnings' : 'Needs you'}</span>
          <Icon name="chevronRight" size={12} />
        </button>
      </Show>
    </Suspense>
  );
}
