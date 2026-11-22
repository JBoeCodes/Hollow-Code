import { For, Show, batch, createEffect, createMemo, createResource, createSignal, on } from 'solid-js';
import { api } from '../api';
import { Button, ErrorText, Skeleton, StatePanel, StatusPill } from './ui';
import { ActionMenu } from './ActionMenu';
import { DiffView } from './DiffView';
import { CHECK_STATE, REVIEW_STATE, checkPhrase, checkState } from '../workState';
import { relativeWhen } from '../homeReturn';
import { flashFooter, openVaultHistory } from '../store';
import { openSessionByKey, selectWorkspaceOpened, workspaceDetail, workspaceId } from '../store/workspaces';
import { draftInWorkspace } from '../store/workspaceDraft';
import { closePreview } from '../store/preview';
import { windowAtMost } from '../windowLayout';
import { canExportMarkdown } from '../resultExport';
import { exportResultMarkdown } from '../store/resultExport';
import { reviewBarModel, reviewedMessage } from '../resultReviewBar';
import { resultKindLabel } from '../resultsCards';
import type { WorkResult } from '../workspaceTypes';
import { errMsg } from '../utils';
import { latestUnlessErrored } from '../stableResource';
import '../styles/results.css';

/**
 * A Result's review bar in the Files tab (COO rebuild L4, 2026-11-10; it was
 * the workspace panel's Results section, ResultsView.tsx). A file in a
 * workspace's `outputs/` with a review record opens with it above the file:
 * its state and checks, where it came from, Mark ready / Accept, Ask for a
 * revision and, after a change, Compare with accepted, which shows the diff
 * in place of the file (`onCompare`). Accepting records the exact content
 * hash the person saw and publishes nothing; only a person accepts (no
 * agent action does). A file with no record shows no bar.
 */
// Reviews in flight and finished, across bar instances: the bar remounts on
// every file switch, so a review started on file A that lands after the
// person went to B and back must still read as busy, and its result must
// reach the new bar (review of L4, Codex).
const [inFlight, setInFlight] = createSignal<ReadonlySet<string>>(new Set());
const [reviewed, setReviewed] = createSignal(0);
const flightKey = (workspace: string, path: string) => `${workspace}\n${path}`;

export function ResultReviewBar(props: {
  workspaceId: string;
  /** Vault-relative, as the review record keys it. */
  path: string;
  /** Bumped when the file was read again (an edit, Refresh). */
  nonce: number;
  /** The file's text as the pane shows it, for the comparison. */
  current: string;
  compare: boolean;
  onCompare: (on: boolean) => void;
}) {
  const [tick, setTick] = createSignal(0);
  // `reviewed()` re-reads after any review lands, so a fetch started before
  // it can't overwrite the new state (a newer fetch supersedes an older one).
  const [list, { mutate }] = createResource(() => [props.workspaceId, props.nonce, tick(), reviewed()] as const, ([id]) => api.workspaceResults(id));
  const result = createMemo(() => (latestUnlessErrored(list) ?? []).find(r => r.path === props.path) ?? null);
  // Busy until this bar has re-read the list after a review landed too: the
  // in-flight key clears and `reviewed` bumps in one batch, so a remounted
  // bar never shows the pre-review state with live buttons.
  const busy = () => inFlight().has(flightKey(props.workspaceId, props.path)) || list.loading;
  const setBusy = (on: boolean, key = flightKey(props.workspaceId, props.path)) => setInFlight(prev => {
    const next = new Set(prev); if (on) next.add(key); else next.delete(key); return next;
  });
  const [error, setError] = createSignal('');
  createEffect(on(() => props.path, () => { setError(''); props.onCompare(false); }, { defer: true }));
  // A file that stopped being a changed Result has nothing to compare.
  createEffect(() => { if (props.compare && !result()?.hasSnapshot) props.onCompare(false); });
  const [snapshot, { refetch: rereadSnapshot }] = createResource(
    () => props.compare && result()?.acceptedSha256 ? [props.path, result()!.acceptedSha256!] as const : null,
    ([path, sha]) => api.resultSnapshot(props.workspaceId, path, sha));
  async function review(state: 'draft' | 'ready' | 'accepted') {
    const r = result();
    if (!r || busy()) return;
    const key = flightKey(props.workspaceId, r.path);
    setBusy(true, key); setError('');
    try {
      const next = await api.setResultReview(props.workspaceId, r.path, state, r.sha256);
      mutate(prev => (prev ?? []).map(x => x.path === next.path ? next : x));
      flashFooter(reviewedMessage(state, next.title));
    } catch (e) {
      const msg = errMsg(e);
      if (msg.startsWith('RESULT_CHANGED')) { setError('This file changed since it was loaded, so nothing was recorded. The new version is shown now; review it before accepting.'); setTick(t => t + 1); }
      else setError(msg);
    } finally { batch(() => { setBusy(false, key); setReviewed(n => n + 1); }); }
  }
  // The request joins the draft once (plan L3 D3), in the Result's own
  // workspace: opened first when another one is on screen.
  const askRevision = async (r: WorkResult) => {
    if (workspaceId() !== props.workspaceId && !await selectWorkspaceOpened(props.workspaceId)) return;
    // In a window too narrow for both, the pane steps aside so the draft is
    // on screen (as the pane's Ask the agent to fix does).
    if (await draftInWorkspace(`Revise ${r.name} (“${r.title}”): `) && windowAtMost('medium')) closePreview();
  };
  // The origin opens the conversation that wrote the file; a narrow window's
  // pane steps aside for it the same way, or the click shows nothing.
  const openOrigin = async (key: string) => {
    if (await openSessionByKey(key) && windowAtMost('medium')) closePreview();
  };
  return <Show when={result()}>{r => {
    const model = () => reviewBarModel(r(), busy());
    const word = () => REVIEW_STATE[r().review];
    const checks = () => checkState(r().checks);
    return <section class="result-review-bar" data-review={r().review} aria-label="Result review">
      <div class="result-states">
        <StatusPill label={word().label} tone={word().tone} hint={word().hint}/>
        <StatusPill label={`Checks from its run: ${checkPhrase(r().checks)}`} tone={CHECK_STATE[checks()].tone} hint={CHECK_STATE[checks()].hint}/>
        <span class="result-origin">{resultKindLabel(r())} · <Show when={r().origin} fallback={<>No run record names this file{r().modified ? ` · changed ${relativeWhen(r().modified)}` : ''}</>}>{o =>
          <>Written by <Show when={o().sessionKey} fallback={<span>a run</span>}><button type="button" class="btn-link" onClick={() => void openOrigin(o().sessionKey!)}>{workspaceDetail()?.sessions.find(s => s.key === o().sessionKey)?.title ?? 'its conversation'}</button></Show> · {relativeWhen(o().finishedAt)}</>}
        </Show></span>
      </div>
      <div class="result-actions">
        <Show when={model().markReady}>{c => <Button variant="primary" size="sm" disabled={c().disabled} title={c().title} onClick={() => void review('ready')}>{c().label}</Button>}</Show>
        <Show when={model().accept}>{c => <Button variant={c().primary ? 'primary' : 'secondary'} size="sm" icon="check" disabled={c().disabled} title={c().title} onClick={() => void review('accepted')}>{c().label}</Button>}</Show>
        <Show when={model().compare}><Button size="sm" icon="compare" aria-pressed={props.compare} onClick={() => props.onCompare(!props.compare)}>{props.compare ? 'Show the file' : 'Compare with accepted'}</Button></Show>
        <Button size="sm" variant="ghost" icon="message" onClick={() => void askRevision(r())}>Ask for a revision</Button>
        <ActionMenu label="More result actions" items={[
          ...(model().backToDraft ? [{ label: 'Move back to draft', action: () => void review('draft') }] : []),
          { label: 'Earlier versions (History & recovery)', action: () => openVaultHistory(r().path) },
          ...(canExportMarkdown(r().name) ? [{ label: 'Export as Markdown…', action: () => void exportResultMarkdown(props.workspaceId, r().path, r().title) }] : []),
        ]}/>
      </div>
      <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
      <Show when={r().review === 'changed'}><p class="result-note">Changed after you accepted it{r().acceptedAt ? ` (${relativeWhen(r().acceptedAt)})` : ''}. The accepted version is kept; this one needs its own review.</p></Show>
      <Show when={r().history.length}>
        <details class="result-history"><summary>Review history <span class="ui-section-count">{r().history.length}</span></summary>
          <ol><For each={r().history}>{h => <li><span>{REVIEW_STATE[(h.state as keyof typeof REVIEW_STATE)]?.label ?? h.state}</span><small>{relativeWhen(h.at)}{h.sha256 ? ` · version ${h.sha256.slice(0, 7)}` : ''}{h.note ? ` · ${h.note}` : ''}</small></li>}</For></ol>
        </details>
      </Show>
      <Show when={props.compare}>
        <div class="result-compare">
          <Show when={!snapshot.error || snapshot.loading} fallback={<StatePanel kind="failed" title="Couldn’t read the accepted version" error={snapshot.error} actions={<><Button size="sm" onClick={() => void rereadSnapshot()}>Retry</Button><Button size="sm" variant="ghost" onClick={() => props.onCompare(false)}>Show the file</Button></>} />}>
            <Show when={latestUnlessErrored(snapshot) != null} fallback={<Skeleton rows={4} label="Loading the accepted version"/>}>
              <DiffView before={latestUnlessErrored(snapshot) ?? ''} after={props.current} beforeLabel="Accepted version" afterLabel="File now"/>
            </Show>
          </Show>
        </div>
      </Show>
    </section>;
  }}</Show>;
}
