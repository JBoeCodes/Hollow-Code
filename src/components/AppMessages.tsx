import { For, Show, createSignal } from 'solid-js';
import { appMessages, type AppMessage } from '../store/appMessages';
import { ErrorText } from './ui';

/** A bounded session history, without replayable actions or live announcements. */
export function AppMessages() {
  return <section class="app-messages" aria-label="App messages">
    <h3>App messages</h3>
    <p class="hint">The last 50 messages from this app session, including dismissed notices.</p>
    <Show when={appMessages().length} fallback={<p class="hint">No app messages this session.</p>}>
      <ol class="app-message-list">
        <For each={appMessages()}>{message => <MessageRow message={message} />}</For>
      </ol>
    </Show>
  </section>;
}

function MessageRow(props: { message: AppMessage }) {
  const [copied, setCopied] = createSignal(false);
  const [error, setError] = createSignal('');
  const copy = async () => {
    setError('');
    try {
      await navigator.clipboard.writeText([props.message.message, props.message.detail].filter(Boolean).join('\n'));
      setCopied(true);
    } catch { setError('Could not copy. Select the message and copy it.'); }
  };
  return <li class="app-message-row">
    <div>
      <span class="app-message-state">{props.message.failed ? 'Failed' : 'Message'}</span>
      <time dateTime={new Date(props.message.at).toISOString()}>{new Date(props.message.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</time>
      <p>{props.message.message}</p>
      <Show when={props.message.detail}>{detail => <details><summary>Details</summary><pre>{detail()}</pre></details>}</Show>
      <Show when={error()}><ErrorText class="app-message-error" text={error()} /></Show>
    </div>
    <button type="button" class="btn small ghost" onClick={() => void copy()}>{copied() ? 'Copied' : 'Copy'}</button>
  </li>;
}
