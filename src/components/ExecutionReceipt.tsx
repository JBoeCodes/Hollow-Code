import { For, Show, Suspense, createMemo, createSignal, createUniqueId, lazy } from 'solid-js';
import {
  canvasesLine,
  compactionsLine,
  effortLine,
  switchLine,
  executionReceiptStateText,
  isOwnReceiptFile,
  phaseTimingRows,
  pullRequestLabel,
  receiptRecoveryNote,
  receiptStatusLabel,
  unbackedClaimLabel,
  type EvidenceRepoFileWire,
  type ExecutionReceipt as ExecutionReceiptData,
  type ReceiptCheckOutcome,
} from '../executionReceipt';
import { api } from '../api';
import { Icon } from '../icons';
import { openVaultHistory, trackTask } from '../store';
import { openWorkspaceResult } from '../store/workspaces';
import { chipTarget } from '../turnFooter';
import type { RepoSidecar } from '../types';
import { errMsg } from '../utils';
import { FlightRecorder } from './FlightRecorder';
import { ErrorText } from './ui';
import { unknownLabel } from '../runOutcome';
import { openFileInSidePane } from '../store/chatRuntime';
import { problemsLabel, runCanvasProblems } from '../canvas/canvasProblems';
import type { RunOutcomeProps } from './RunOutcome';

// R5: the Cites row loads only for a receipt with cites (App chunk budget).
const ReceiptCites = lazy(() => import('./ReceiptCites'));
const RunOutcome = lazy(() => import('./RunOutcome').then(m => ({ default: m.RunOutcome })));
const ReceiptCheckActions = lazy(() => import('./ReceiptCheckActions'));

export interface ExecutionReceiptProps {
  receipt: ExecutionReceiptData;
  defaultOpen?: boolean;
  /** Hides output excerpts while retaining paths, commands, and outcomes. */
  compact?: boolean;
  class?: string;
  /** Chat replies leave recall to the side panel's summary card (RS1); an
   *  automation's run record still lists it. */
  hideRecall?: boolean;
  /** Optional Chat-only feedback hook. The receipt stays presentation-only;
   * callers decide how to open a correction proposal. */
  onRecallFeedback?: (item: NonNullable<ExecutionReceiptData['recall']>[number], action: 'notRelevant' | 'outdated') => void;
  /** The workspace whose conversation this is: a changed path in its
   *  `outputs/` opens in its Results, like the reply's file chip (EH4,
   *  `turnFooter.ts` `chipTarget`). Absent outside one. */
  workspace?: { id: string; kind?: string } | null;
  head?: Omit<RunOutcomeProps, 'receipt'>;
  /** Chat only: append a request to its owned draft without sending. */
  onAskRerun?: (text: string) => void;
  onOpenOutput?: (path: string) => void;
}

/** A compact, progressively disclosed account of observed writes and checks. */
export function ExecutionReceipt(props: ExecutionReceiptProps) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(!!props.defaultOpen);
  const statusLabel = () => receiptStatusLabel(props.receipt.status);
  const stateText = () => executionReceiptStateText(props.receipt);
  // Its own changes only: another chat's (`shared`) are listed, not counted.
  const changedPathCount = () => props.receipt.files.filter(isOwnReceiptFile).length;
  const recallCount = () => props.hideRecall ? 0 : (props.receipt.recall?.length ?? 0);
  // Plan completion L2: what the run did beyond its files (`run_effects`).
  const outside = () => props.receipt.outsideEffects;
  const outsideCount = () => (outside()?.recorded ?? []).filter(e => e.outcome !== 'failed').length + (outside()?.moreRecorded ?? 0);
  const outsideLabelId = `${id}-outside`;
  const repoChanges = () => props.receipt.repoChanges ?? [];
  const repoFileCount = () => props.receipt.repoFileCount ?? 0;
  // Paperclip PR 16: patches load once, on demand, from the run's sidecar.
  const [patches, setPatches] = createSignal<RepoSidecar | null | undefined>(undefined);
  const [patchError, setPatchError] = createSignal<string | null>(null);
  const [loadingPatches, setLoadingPatches] = createSignal(false);
  const loadPatches = async () => {
    const runId = props.receipt.historyRunId;
    if (!runId || loadingPatches()) return;
    setLoadingPatches(true);
    setPatchError(null);
    try {
      setPatches(await trackTask('Load repository patches', () => api.readHistoryRepoChanges(runId)));
    } catch (e) {
      setPatchError(errMsg(e));
    } finally {
      setLoadingPatches(false);
    }
  };
  const patchFor = (root: string, path: string) =>
    patches()?.repos.find(repo => repo.root === root)?.files.find(file => file.path === path)?.patch;
  const countSummary = createMemo(() => {
    const parts: string[] = [];
    if (changedPathCount() > 0) parts.push(plural(changedPathCount(), 'change'));
    if (repoFileCount() > 0) parts.push(plural(repoFileCount(), 'repo change'));
    if (props.receipt.checks.length > 0) parts.push(plural(props.receipt.checks.length, 'check'));
    if (recallCount() > 0) parts.push(`${recallCount()} recalled`);
    if (outsideCount() > 0) parts.push(plural(outsideCount(), 'outside action'));
    return parts.join(' · ') || 'No file changes';
  });

  const filesLabelId = `${id}-files`;
  const checksLabelId = `${id}-checks`;
  const failuresLabelId = `${id}-failures`;
  const recallLabelId = `${id}-recall`;
  const repoLabelId = `${id}-repo`;
  const timingsLabelId = `${id}-timings`;
  // Paperclip PR 15c: measured phases only; hidden in the compact receipt.
  const timingRows = createMemo(() => phaseTimingRows(props.receipt.phaseTimings));

  return (
    <div class="run-record">
    <Show when={!props.compact}>
      <Suspense fallback={<p class="toolcall-result">{stateText()}</p>}>
        <RunOutcome {...props.head} receipt={props.receipt} />
      </Suspense>
    </Show>
    <details
      class={`toolcalls execution-receipt ${props.class ?? ''}`}
      classList={{
        'receipt-verified': props.receipt.status === 'verified',
        'receipt-failed': props.receipt.status === 'failed',
        'receipt-unverified': props.receipt.status === 'unverified' || props.receipt.status === 'stale',
      }}
      open={open()}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary
        class="toolcall-head execution-receipt-summary"
        aria-label={`Run record: ${statusLabel()}. ${countSummary()}. ${stateText()}${unbackedClaimLabel(props.receipt.unbackedClaim, props.receipt.claimKind) ? ` ${unbackedClaimLabel(props.receipt.unbackedClaim, props.receipt.claimKind)}.` : ''}`}
      >
        <span class="toolcall-icon" aria-hidden="true">{statusGlyph(props.receipt.status)}</span>
        <span class="toolcall-verb">Run record</span>
        <span class="toolcall-target">{countSummary()}</span>
        <span class="toolcall-spacer" />
        <span class="toolcall-status">{statusLabel()}</span>
        <span class="toolcall-caret" aria-hidden="true"><Icon name={open() ? 'chevronDown' : 'chevronRight'} size={12} /></span>
      </summary>

      <div class="toolcall-body execution-receipt-body">
        <p class="toolcall-result" role="status">
          {stateText()}
          <Show when={unbackedClaimLabel(props.receipt.unbackedClaim, props.receipt.claimKind)}>
            {(label) => (
              <>
                {' '}
                <span
                  class="execution-receipt-claim"
                  title={props.receipt.claimKind === 'done' || props.receipt.claimKind === 'fixed'
                    ? 'The final reply says the work is finished, but nothing recorded confirms it.'
                    : "The final reply says its checks pass, but the checks this run recorded don't back that."}
                >
                  {label()}
                </span>
              </>
            )}
          </Show>
        </p>

        <Show when={props.receipt.files.length > 0}>
          <div class="toolcall-section-label" id={filesLabelId}>Changed</div>
          <ul class="execution-receipt-list" aria-labelledby={filesLabelId}>
            <For each={props.receipt.files}>
              {(file) => {
                // A Result opens with its review bar, as its chip does; its change
                // stays one click away on the reply's More actions.
                const target = () => chipTarget({ path: file.path, changeType: file.changeType ?? 'modify' }, props.workspace);
                return (
                <li class="execution-receipt-row">
                  <button
                    type="button"
                    class="panel-link execution-receipt-path"
                    aria-label={props.onOpenOutput && file.changeType !== 'delete' ? `Open ${file.path}` : (target().to === 'results' ? `Review ${file.path}` : `Open ${file.path} in History & recovery`)}
                    title={props.onOpenOutput && file.changeType !== 'delete' ? 'Open file' : target().to === 'results' ? 'Open with its review bar' : 'Open in History & recovery'}
                    onClick={() => {
                      if (props.onOpenOutput && file.changeType !== 'delete') { props.onOpenOutput(file.path); return; }
                      const t = target();
                      if (t.to === 'results') void openWorkspaceResult(t.workspaceId, t.path);
                      else openVaultHistory(file.path, props.receipt.historyRunId);
                    }}
                  >
                    <code class="toolcall-target mono">{file.path}</code>
                  </button>
                  <Show when={props.onOpenOutput && file.changeType !== 'delete'}>
                    <button type="button" class="btn ghost sm" aria-label={`Open ${file.path} in History & recovery`}
                      onClick={() => openVaultHistory(file.path, props.receipt.historyRunId)}>History</button>
                  </Show>
                  <span class="toolcall-extra">
                    <Show when={file.changeType}> · {file.changeType}</Show>
                    {' · '}{file.tool}
                    <Show when={file.notRecoverable}> · not recoverable</Show>
                  </span>
                </li>
                );
              }}
            </For>
            <Show when={receiptRecoveryNote(props.receipt)}>
              {(note) => <li class="execution-receipt-row">{note()}</li>}
            </Show>
            <Show when={(props.receipt.unattributedFileCount ?? 0) > 0}>
              <li class="execution-receipt-row">
                {plural(props.receipt.unattributedFileCount ?? 0, 'path')} came from filesystem history without a matching structured write.
              </li>
            </Show>
          </ul>
        </Show>

        <Show when={repoChanges().length > 0}>
          <div class="toolcall-section-label" id={repoLabelId}>Repository changes</div>
          <Show when={!props.compact && props.receipt.historyRunId && patches() === undefined}>
            <button
              type="button"
              class="btn ghost sm execution-receipt-patches"
              disabled={loadingPatches()}
              onClick={() => void loadPatches()}
            >
              {loadingPatches() ? 'Loading patches…' : 'Show patches'}
            </button>
          </Show>
          <Show when={patchError()}>
            <ErrorText class="toolcall-result error" text={`Patches could not be loaded: ${patchError()}`} />
          </Show>
          <Show when={patches() === null}>
            <p class="toolcall-result">No patches were stored for this run.</p>
          </Show>
          <For each={repoChanges()}>
            {(repo) => (
              <div class="execution-receipt-repo" aria-labelledby={repoLabelId}>
                <div class="execution-receipt-row">
                  <span class="toolcall-verb" title={repo.root}>{repo.label}</span>
                  <Show when={repo.headBefore || repo.headAfter}>
                    <span class="toolcall-extra mono">
                      {' · '}{shortHead(repo.headBefore)}
                      <Show when={repo.headAfter !== repo.headBefore}> → {shortHead(repo.headAfter)}</Show>
                    </span>
                  </Show>
                  <Show when={repo.status !== 'captured'}>
                    <span class="toolcall-extra"> · not recorded{repo.error ? `: ${repo.error}` : ''}</span>
                  </Show>
                </div>
                <ul class="execution-receipt-list" aria-label={`Changes in ${repo.label}`}>
                  <For each={repo.files}>
                    {(file) => (
                      <li class="execution-receipt-row">
                        <Show when={repo.root.startsWith('/') && !file.withheld && !file.path.startsWith('/') && !file.path.split('/').includes('..')}
                          fallback={<code class="toolcall-target mono">{file.path}</code>}>
                          <button type="button" class="panel-link execution-receipt-path" aria-label={`Preview ${file.path}`}
                            onClick={() => openFileInSidePane(`${repo.root.replace(/\/$/, '')}/${file.path}`)}><code class="toolcall-target mono">{file.path}</code></button>
                        </Show>
                        <span class="toolcall-extra">
                          {' · '}{file.changeType}{repoCounts(file)}{repoFlags(file)}
                          {' · '}{file.tool ?? 'filesystem observation'}
                        </span>
                        <Show when={!props.compact && patches()}>
                          <Show
                            when={!file.withheld}
                            fallback={<p class="toolcall-extra">Its content was not stored (a secret-named file).</p>}
                          >
                            <Show when={patchFor(repo.root, file.path)}>
                              {(patch) => (
                                <pre class="toolcall-result execution-receipt-patch"><code>{patch()}</code></pre>
                              )}
                            </Show>
                          </Show>
                        </Show>
                      </li>
                    )}
                  </For>
                  <Show when={repo.totalFiles > repo.files.length}>
                    <li class="execution-receipt-row">
                      {repo.totalFiles - repo.files.length} more in the run record
                    </li>
                  </Show>
                </ul>
              </div>
            )}
          </For>
        </Show>

        <Show when={props.receipt.checks.length > 0}>
          <div class="toolcall-section-label" id={checksLabelId}>Checks</div>
          <ul class="execution-receipt-list" aria-labelledby={checksLabelId}>
            <For each={props.receipt.checks}>
              {(check) => (
                <li class="execution-receipt-row">
                  <div>
                    <span class="toolcall-verb">{checkKindLabel(check.kind)}</span>
                    <span class="toolcall-extra"> · {checkOutcomeLabel(check.outcome)}</span>
                  </div>
                  <Suspense fallback={<code class="toolcall-result">{check.command.slice(0, 320)}</code>}>
                    <ReceiptCheckActions command={check.command} onAskRerun={props.onAskRerun} />
                  </Suspense>
                  <Show when={!props.compact && check.excerpt}>
                    <pre class="toolcall-result" classList={{ error: check.outcome === 'failed' }}>
                      <code>{check.excerpt}</code>
                    </pre>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={props.receipt.failures.length > 0}>
          <div class="toolcall-section-label" id={failuresLabelId}>Failures</div>
          <ul class="execution-receipt-list" aria-labelledby={failuresLabelId}>
            <For each={props.receipt.failures}>
              {(failure) => (
                <li class="execution-receipt-row">
                  <div class="toolcall-verb">{failure.tool}</div>
                  <Show
                    when={!props.compact}
                    fallback={<span class="toolcall-extra"> · failed</span>}
                  >
                    <pre class="toolcall-result error"><code>{failure.excerpt}</code></pre>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={recallCount() > 0}>
          <div class="toolcall-section-label" id={recallLabelId}>Recall used</div>
          <ul class="execution-receipt-list" aria-labelledby={recallLabelId}>
            <For each={props.receipt.recall}>
              {(item) => (
                <li class="execution-receipt-row">
                  <div class="toolcall-verb">{item.subject}</div>
                  <div class="toolcall-result">{item.statement}</div>
                  <Show when={item.lifecycle}>
                    {(label) => <div class="toolcall-extra">{label()}</div>}
                  </Show>
                  <code class="toolcall-extra mono">{item.path}</code>
                  <Show when={props.onRecallFeedback}>
                    <div class="receipt-recall-actions">
                      <button type="button" class="btn ghost sm" onClick={() => props.onRecallFeedback?.(item, 'notRelevant')}>Not relevant</button>
                      <button type="button" class="btn ghost sm" onClick={() => props.onRecallFeedback?.(item, 'outdated')}>Outdated</button>
                    </div>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={(outside()?.recorded?.length ?? 0) > 0 || (outside()?.unknown?.length ?? 0) > 0}>
          <div class="toolcall-section-label" id={outsideLabelId}>Outside its files</div>
          <ul class="execution-receipt-list" aria-labelledby={outsideLabelId}>
            <For each={(outside()?.recorded ?? []).slice(0, 50)}>
              {(effect) => (
                <li class="execution-receipt-row execution-receipt-outside" data-outcome={effect.outcome}>
                  <span class="toolcall-verb">{effect.label}</span>
                  <span class="toolcall-extra"> · {effect.outcome === 'done' ? 'done' : effect.outcome === 'failed' ? 'failed, nothing sent' : 'may have happened'}</span>
                </li>
              )}
            </For>
            <Show when={(outside()?.moreRecorded ?? 0) > 0}>
              <li class="execution-receipt-row">{outside()!.moreRecorded} more outside actions were recorded.</li>
            </Show>
            <For each={(outside()?.unknown ?? []).slice(0, 20)}>
              {(group) => (
                <li class="execution-receipt-row">
                  {group.count} {unknownLabel(group.label, group.count)} whose effects Hollow can’t see
                  <Show when={!props.compact && (group.examples?.length ?? 0) > 0}>
                    <span class="toolcall-extra mono"> · {group.examples!.slice(0, 3).join(' · ')}</span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={props.receipt.effort}>
          {(effort) => (
            <p class="execution-receipt-row execution-receipt-effort">
              <span class="toolcall-verb">Effort</span>
              <span class="toolcall-extra mono"> · {effortLine(effort())}</span>
              <Show when={effort().note}><span class="toolcall-extra"> · {effort().note}</span></Show>
            </p>
          )}
        </Show>

        <Show when={props.receipt.pullRequests?.length}>
          <p class="execution-receipt-row execution-receipt-prs">
            <span class="toolcall-verb">Pull requests</span>
            <For each={props.receipt.pullRequests}>{(p) => (
              <span class="toolcall-extra"> · <Show when={p.url} fallback={pullRequestLabel(p)}>{(url) => (
                <a href={url()}>{pullRequestLabel(p)}</a>
              )}</Show></span>
            )}</For>
          </p>
        </Show>

        <Show when={props.receipt.scheduledWake}>
          {(line) => (
            <p class="execution-receipt-row execution-receipt-wake">
              <span class="toolcall-verb">Wake-up</span>
              <span class="toolcall-extra"> · {line()}</span>
            </p>
          )}
        </Show>

        <Show when={canvasesLine(props.receipt.canvases)}>
          {(line) => (
            <p class="execution-receipt-row execution-receipt-canvases">
              <span class="toolcall-verb">Canvases</span>
              <span class="toolcall-extra mono"> · {line()}</span>
              {/* Repo ideas 2026-10-27 #1: the findings row's count, while the reply's canvas is on screen. */}
              <Show when={(() => { const c = runCanvasProblems(props.receipt.runId); return c && problemsLabel(c.layout, c.design, c.more); })()}>
                {(problems) => <span class="toolcall-extra"> · {problems()}</span>}
              </Show>
            </p>
          )}
        </Show>

        <Show when={compactionsLine(props.receipt.compactions)}>
          {(line) => (
            <p
              class="execution-receipt-row execution-receipt-compactions"
              title="The agent summarized its own history to make room. In a chat, Hollow sends it the conversation's task state (goal, rules, steps done) again with the next message."
            >
              <span class="toolcall-verb">Compacted</span>
              <span class="toolcall-extra"> · {line()}</span>
            </p>
          )}
        </Show>

        {/* Its own Suspense: loading the row must never suspend the transcript. */}
        <Show when={props.receipt.cites}>{(cites) => <Suspense><ReceiptCites cites={cites()} /></Suspense>}</Show>

        <For each={props.receipt.accountChoices ?? []}>
          {reason => <p class="execution-receipt-row"><span class="toolcall-verb">Account choice</span><span class="toolcall-extra"> · {reason}</span></p>}
        </For>
        <For each={props.receipt.switches ?? []}>
          {(switched) => (
            <p class="execution-receipt-row execution-receipt-switch">
              <span class="toolcall-verb">{switched.kind === 'agent' ? 'Moved agent' : switched.kind === 'login' ? 'Stayed on Claude Code login' : switched.kind === 'retry' ? 'Retried once' : switched.kind === 'rested' ? 'Started fresh' : 'Switched account'}</span>
              <Show when={switched.kind !== 'login' && switched.kind !== 'retry' && switched.kind !== 'rested'}><span class="toolcall-extra mono"> · {switchLine(switched)}</span></Show>
              <span class="toolcall-extra"> · {switched.reason}</span>
            </p>
          )}
        </For>

        {/* Flight recorder (master plan #15): the turn's steps, loaded when opened. */}
        <Show when={!props.compact && props.receipt.historyRunId}>
          {(runId) => {
            const [shown, setShown] = createSignal(props.receipt.status === 'failed');
            return (
              <details class="flight-recorder-wrap" open={shown()} onToggle={(e) => setShown(e.currentTarget.open)}>
                <summary class="toolcall-section-label">Timeline</summary>
                <Show when={shown()}><FlightRecorder runId={runId()} /></Show>
              </details>
            );
          }}
        </Show>

        <Show when={!props.compact && timingRows().length > 0}>
          <div class="toolcall-section-label" id={timingsLabelId}>Timings</div>
          <ul class="execution-receipt-list execution-receipt-timings" aria-labelledby={timingsLabelId}>
            <For each={timingRows()}>
              {(row) => (
                <li class="execution-receipt-row">
                  <span class="toolcall-verb">{row.label}</span>
                  <span class="toolcall-extra mono"> · {row.value}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </details>
    </div>
  );
}

function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? '' : 's'}`;
}

function shortHead(head: string | null | undefined): string {
  return head ? head.slice(0, 7) : 'no commit';
}

function repoCounts(file: EvidenceRepoFileWire): string {
  if (file.binary || file.additions == null || file.deletions == null) return '';
  return ` +${file.additions}/−${file.deletions}`;
}

function repoFlags(file: EvidenceRepoFileWire): string {
  const flags: string[] = [];
  if (file.binary) flags.push('binary');
  if (file.oversized) flags.push('too large to show');
  if (file.patchTruncated) flags.push('patch truncated');
  if (file.withheld) flags.push('content not stored');
  if (file.gitlink) flags.push('submodule');
  if (file.status === 'special') flags.push('not a regular file');
  if (file.dirtyBefore) flags.push('had earlier uncommitted edits');
  return flags.length > 0 ? ` (${flags.join(', ')})` : '';
}

function checkOutcomeLabel(outcome: ReceiptCheckOutcome): string {
  switch (outcome) {
    case 'passed': return 'passed';
    case 'failed': return 'failed';
    case 'unknown': return 'outcome unavailable';
  }
}

function checkKindLabel(kind: ExecutionReceiptData['checks'][number]['kind']): string {
  switch (kind) {
    case 'test': return 'Test';
    case 'build': return 'Build';
    case 'typecheck': return 'Typecheck';
    case 'lint': return 'Lint';
    case 'check': return 'Check';
    case 'gate': return 'Gate';
  }
}

function statusGlyph(status: ExecutionReceiptData['status']): string {
  switch (status) {
    case 'verified': return '✓';
    case 'failed': return '✕';
    case 'stale': return '◷';
    case 'unverified': return '?';
    case 'no_changes': return '–';
  }
}
