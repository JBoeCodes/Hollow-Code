import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { appendToChatDraft } from '../store/chatRuntime';
import { openInBrowser } from '../store';
import { errMsg, openExternal } from '../utils';
import { ErrorText, StatePanel } from './ui';
import { checksSummary, commentDraft, mergeWords, prStateWords, reviewStateWords, reviewWords, safeLink, type PrView } from '../prPanelModel';
import '../styles/pr-panel.css';

/** The PR tab (parity 5.4, D2 and D6): the pull request for the chat folder's
 *  branch, its checks and its comments, read through the GitHub CLI. Read-only:
 *  Hollow creates, comments on and merges nothing here; links open GitHub in the
 *  browser and "Ask about this" only drafts a message. */
export function PrPanel(props: { cwd: string }) {
  const [view, setView] = createSignal<PrView | null>(null);
  const [error, setError] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const reads = latestRequest();
  async function load() {
    const gen = reads.start();
    setLoading(true); setError('');
    try {
      const next = await api.prCardView(props.cwd);
      if (reads.isLatest(gen)) setView(next);
    } catch (e) { if (reads.isLatest(gen)) setError(errMsg(e)); }
    finally { if (reads.isLatest(gen)) setLoading(false); }
  }
  onMount(() => void load());
  onCleanup(() => reads.cancel());
  const open = (url: string | null) => { const safe = safeLink(url); if (safe) openInBrowser(safe).catch(() => openExternal(safe)); };

  return (
    <div class="pr-panel">
      <div class="pr-bar">
        <span class="pr-title-line"><Icon name="gitPullRequest" size={15} /> Pull request</span>
        <button type="button" class="btn icon" aria-label="Refresh the pull request" title="Refresh the pull request" disabled={loading()} onClick={() => void load()}><Icon name="refresh" size={14} /></button>
      </div>
      <Show when={error()}>{msg => <ErrorText class="bc-error" as="div" text={msg()} />}</Show>
      <Show when={loading() && !view()}><p class="bc-muted" role="status">Asking GitHub…</p></Show>
      <Show when={view()?.note}>{note => <StatePanel compact kind="empty" title="No pull request to show">{note()}</StatePanel>}</Show>
      <Show when={view()?.detail}>{pr => {
        const checks = () => checksSummary(pr().checks);
        return <>
          <header class="pr-head">
            <h2><button type="button" class="pr-link" onClick={() => open(pr().url)} title={pr().url}>#{pr().number} {pr().title}</button></h2>
            <p class="pr-meta">
              <span class="pr-chip" data-state={pr().draft ? 'draft' : pr().state}>{prStateWords(pr())}</span>
              <span>{pr().head} → {pr().base}</span>
              <Show when={reviewWords(pr().reviewDecision)}>{w => <span class="pr-chip" data-review={pr().reviewDecision}>{w()}</span>}</Show>
              <Show when={pr().state === 'open' && mergeWords(pr().mergeState)}>{w => <span class="bc-muted">{w()}</span>}</Show>
            </p>
          </header>
          <section class="pr-section" aria-label="Checks">
            <h3>Checks <span class="pr-chip" data-checks={checks().worst}>{checks().words}</span></h3>
            <ul class="pr-checks">
              <For each={pr().checks}>{ch => (
                <li data-state={ch.state}>
                  <span class="pr-dot" data-state={ch.state} role="img" aria-label={ch.state} />
                  <Show when={safeLink(ch.url)} fallback={<span>{ch.name}</span>}>{u => <button type="button" class="pr-link" onClick={() => open(u())} title={u()}>{ch.name}</button>}</Show>
                </li>
              )}</For>
            </ul>
            <Show when={pr().moreChecks}><p class="bc-muted">{pr().moreChecks} more checks not listed.</p></Show>
          </section>
          <section class="pr-section" aria-label="Comments">
            <h3>Comments <span class="bc-muted">{pr().comments.length + pr().moreComments}</span></h3>
            <Show when={pr().moreComments}><p class="bc-muted">The {pr().moreComments} oldest are on GitHub.</p></Show>
            <Show when={pr().comments.length} fallback={<p class="bc-muted">No comments or reviews yet.</p>}>
              <ul class="pr-comments">
                <For each={pr().comments}>{c => (
                  <li>
                    <div class="pr-comment-head">
                      <strong>{c.author}</strong>
                      <Show when={c.kind === 'review'}><span class="pr-chip" data-review={c.state ?? ''}>{reviewStateWords(c.state)}</span></Show>
                      <time class="bc-muted" datetime={c.createdAt}>{c.createdAt ? new Date(c.createdAt).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : ''}</time>
                    </div>
                    <Show when={c.body}><p class="pr-comment-body">{c.body}</p></Show>
                    <div class="pr-comment-actions">
                      <Show when={c.body}><button type="button" class="btn small ghost" title="Add this comment to your message (nothing is sent)" onClick={() => appendToChatDraft(commentDraft(pr(), c))}>Ask about this</button></Show>
                      <Show when={safeLink(c.url)}>{u => <button type="button" class="btn small ghost" onClick={() => open(u())}>Open on GitHub</button>}</Show>
                    </div>
                  </li>
                )}</For>
              </ul>
            </Show>
          </section>
        </>;
      }}</Show>
    </div>
  );
}
