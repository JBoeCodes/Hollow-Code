import { Show } from 'solid-js';
import { Button } from './ui';

/** The words every surface uses when a refresh failed and the list on screen
 *  is the last one that loaded (X21). Home's "Not refreshed" row says the same
 *  thing for its cards. */
export const REFRESH_FAILED_TEXT = 'Couldn’t refresh — showing what was here before.';

/**
 * One line above a list that a `preserveDuringRefetch` resource feeds, shown
 * only while its last refresh failed (`refreshFailed`), with Retry. Without it
 * a failed refresh leaves the old list on screen looking current.
 */
export function RefreshNotice(props: {
  resource: { readonly refreshFailed: boolean; readonly loading: boolean };
  onRetry: () => unknown;
  /** What the list is, for screen readers: "automations", "skills"… */
  what: string;
  class?: string;
}) {
  return (
    <Show when={props.resource.refreshFailed}>
      <div class={`refresh-notice${props.class ? ` ${props.class}` : ''}`} role="status" aria-label={`Couldn’t refresh ${props.what}`}>
        <span>{REFRESH_FAILED_TEXT}</span>
        <Button size="sm" disabled={props.resource.loading} onClick={() => void props.onRetry()}>{props.resource.loading ? 'Retrying…' : 'Retry'}</Button>
      </div>
    </Show>
  );
}
