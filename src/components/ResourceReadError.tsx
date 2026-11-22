import { Show } from 'solid-js';
import { ErrorText } from './ui';
import { errMsg } from '../utils';

/** A read failure keeps its retry beside the data that failed, including stale data. */
export function ResourceReadError(props: {
  label: string;
  read: { readonly error: unknown; readonly loading: boolean; readonly refreshFailed?: boolean };
  retry: () => unknown;
}) {
  return <Show when={props.read.error}>{error => <ErrorText class="settings-row-warn" text={`Couldn't read ${props.label}: ${errMsg(error())}`}>
    {' '}{props.read.refreshFailed ? 'Showing what was here before. ' : ''}
    <button type="button" class="btn-link" disabled={props.read.loading} onClick={() => void props.retry()}>{props.read.loading ? 'Retrying…' : 'Retry'}</button>
  </ErrorText>}</Show>;
}
