import { For, Show, createMemo, createSignal } from 'solid-js';
import type { ExecutionReceipt } from '../executionReceipt';
import { chatCwd, openFileInSidePane, vaultRoot } from '../store/chat';
import { chatAgentSessionKey } from '../store';
import { replyVideos, splitReplyVideos } from '../replyMedia';
import { MediaPreview } from './MediaPreview';

/**
 * The videos under a finished reply (inline video, 2026-10-17): the ones its
 * receipt says it made or changed (a render through Bash included), then the
 * ones its text names. At most three players; the rest are a list that opens
 * the side panel. Mounted outside the reply's Markdown (whose DOM is rebuilt
 * on every change) and outside the run fold, so it shows on settled replies
 * only. A player mounts only on screen and within the app-wide budget
 * (`MediaPreview`); until then it is a card with a Play button.
 */
export function ReplyMedia(props: { receipt?: ExecutionReceipt; text: string }) {
  // A plain signal, not a resource: reading a resource here would suspend the
  // transcript's Suspense boundary on every settled reply.
  const [vault, setVault] = createSignal<string | null>(null);
  void vaultRoot().then(setVault);
  const videos = createMemo(() =>
    splitReplyVideos(replyVideos(props.receipt, props.text, chatCwd() || vault(), vault())));
  const name = (p: string) => p.slice(p.lastIndexOf('/') + 1);
  return (
    <Show when={videos().shown.length > 0}>
      <div class="reply-media" role="group" aria-label="Videos from this reply">
        <For each={videos().shown}>
          {(path) => (
            <MediaPreview variant="inline" path={path} sessionKey={chatAgentSessionKey()} onOpenPane={() => openFileInSidePane(path)} />
          )}
        </For>
        <Show when={videos().more.length > 0}>
          <details class="reply-media-more">
            <summary>+{videos().more.length} more video{videos().more.length === 1 ? '' : 's'}</summary>
            <ul role="list">
              <For each={videos().more}>
                {(path) => (
                  <li>
                    <button type="button" class="btn-link" title={path} onClick={() => openFileInSidePane(path)}>{name(path)}</button>
                  </li>
                )}
              </For>
            </ul>
          </details>
        </Show>
      </div>
    </Show>
  );
}
