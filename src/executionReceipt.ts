import { formatDuration } from './formatTime';

export type ExecutionReceiptStatus =
  | 'verified'
  | 'failed'
  | 'stale'
  | 'unverified'
  | 'no_changes';

export type ReceiptOrdering = 'known' | 'unknown';
export type ReceiptCheckKind = 'test' | 'build' | 'typecheck' | 'lint' | 'check' | 'gate';
export type ReceiptCheckOutcome = 'passed' | 'failed' | 'unknown';

export interface ReceiptFile {
  path: string;
  tool: string;
  sequence: number;
  excerpt?: string;
  /** Backend filesystem classification when this came from a durable receipt. */
  changeType?: string;
  /** Whether the path was named by a tool or discovered by the filesystem diff;
   *  `shared` when another chat in the same workspace folder changed it. */
  attribution?: 'confirmed' | 'observed' | 'shared' | string;
  /** Lines added / removed, when the backend could count them. */
  additions?: number;
  deletions?: number;
  /** History can't restore this file (durable receipts, merged plan slice 4). */
  notRecoverable?: boolean;
}

export interface ReceiptCheck {
  command: string;
  kind: ReceiptCheckKind;
  outcome: ReceiptCheckOutcome;
  sequence: number;
  excerpt?: string;
}

export interface ReceiptFailure {
  tool: string;
  sequence: number;
  excerpt: string;
}

/** Durable evidence receipt emitted by the Rust backend (camelCase wire shape). */
export interface EvidenceReceiptWire {
  id: string;
  runId: string;
  source: string;
  sourceId: string | null;
  startedAt: string;
  finishedAt: string;
  status: string;
  verificationState: string;
  files: EvidenceReceiptFileWire[];
  checks: EvidenceReceiptCheckWire[];
  failures: EvidenceReceiptFailureWire[];
  historyRunId: string;
  /** Hollow-owned session key. Optional: receipts written before agent
   *  sessions shipped have no key, and `sourceId` (the HARNESS session id)
   *  is still null whenever a first turn died before its stream announced
   *  one — which is exactly the receipt this field makes findable. */
  sessionKey?: string | null;
  /** Whether ending the run's processes left nothing Hollow could find
   *  (`verified`) or not (`unconfirmed`). Absent on receipts from before
   *  process-group cleanup and on runs without an owned process group. The
   *  receipt's failures already carry the human wording. */
  processStop?: 'verified' | 'unconfirmed' | null;
  /** Paperclip PR 14a: how the run ended (`src/resultKind.ts` labels it).
   *  Absent on older receipts and on surfaces that do not classify. A
   *  failure kind also adds a `result` line to `failures`. */
  resultKind?: string | null;
  /** Changes in the run's target git repositories (Paperclip PR 16), without
   *  patches (those load on demand from the run's sidecar through
   *  `readHistoryRepoChanges`). Absent when the run had no repository target. */
  repoChanges?: EvidenceRepoChangesWire[];
  /** Paperclip PR 15c: where the run's time went. Absent on older receipts;
   *  an unmeasured phase is omitted, never 0. */
  phaseTimings?: PhaseTimingsWire | null;
  /** Agent effectiveness F11: the run's context manifest (schema, size,
   *  SHA-256); `runContextManifest` reads it. Absent when none was kept. */
  contextManifest?: { schemaVersion: number; bytes: number; sha256: string } | null;
  /** Merged plan Track U: the effort the turn asked for and what its agent
   *  got (`evidence::ReceiptEffort`). Absent when none was asked for. */
  effort?: ReceiptEffortWire | null;
  /** COO rebuild L7: what the CFO decided (agent, model, effort and who
   *  set it, the job kind and its reason). */
  cfo?: import('./generated/ipc').CfoDecision | null;
  /** Merged plan decision 1: the services this turn started through
   *  `hollow_services` (`evidence::ReceiptService`). Absent when none. */
  services?: ReceiptServiceWire[];
  /** Merged plan decision 6: where the run moved to keep working at a
   *  ceiling (`evidence::ReceiptSwitch`). Absent when it never moved. */
  switches?: ReceiptSwitchWire[];
  accountChoices?: string[];
  /** Merged plan slice 4 (decision 5): whether History can restore every
   *  vault change listed. False once one file can't; absent when unknown,
   *  when no vault file changed, and on older receipts. */
  historyRecoverable?: boolean | null;
  /** Canvas C3 wave 2 C4: the reply's `hollow-canvas` fences and their body
   *  bytes (`evidence::ReceiptCanvases`). Absent when it drew none. */
  canvases?: ReceiptCanvasesWire | null;
  /** Overnight C3: the wake-up this turn asked for (`sessions.wake_me`), as
   *  one line. Absent when none is pending. */
  scheduledWake?: string | null;
  /** Overnight F8: pull requests this turn merged or opened
   *  (`evidence::ReceiptPullRequest`). Absent when none. */
  pullRequests?: ReceiptPullRequest[] | null;
  /** Plan completion L2 (`run_effects::OutsideEffects`): what the run did
   *  beyond its files, and the calls Hollow can't see into. Absent when it
   *  had neither, and on receipts written before it (unknown, not none). */
  outsideEffects?: OutsideEffectsWire | null;
  /** OF18: the tool calls the run's stream reported, errors included.
   *  Absent (unknown, not 0) on receipts written before it and for an agent
   *  that reports no tool calls (Antigravity). */
  toolCalls?: number;
  /** S13: a Claude run's completed Write/Edit/MultiEdit calls into Claude
   *  Code's own auto-memory folder. Absent for other agents and on receipts
   *  written before it. */
  memoryWrites?: number;
  /** OB22: how many times the agent compacted its context in the run (the
   *  `compact_boundary` events of a Claude-format stream: Claude, Grok),
   *  counted for chat and automation runs only, not heartbeat or delegate
   *  runs. Absent when none, for Codex and Antigravity, and on receipts
   *  written before it. */
  compactions?: number;
  /** S27: whether the turn's final reply claimed its checks pass, against
   *  the checks this receipt saw (`evidence::ReceiptVerification`). Absent
   *  on surfaces with no reply and on receipts written before it. */
  verification?: ReceiptVerificationWire | null;
  /** Repo features R5: the final reply's `path:line` cites, checked when the
   *  run ended (`cite_check::ReceiptCites`). Absent when it cited none. */
  cites?: ReceiptCitesWire | null;
}

/** R5 (`cite_check::ReceiptCites`, camelCase): each count is omitted when 0;
 *  `items` lists the problem cites (any status but `seen`), at most 16,
 *  never one under a `restricted` folder. */
export interface ReceiptCitesWire {
  checked: number;
  seen?: number;
  fileOnly?: number;
  earlier?: number;
  notOpened?: number;
  unknown?: number;
  pastEnd?: number;
  missing?: number;
  unresolved?: number;
  items?: ReceiptCiteWire[];
}

/** One problem cite (`cite_check::ReceiptCite`); `status` is a count key of
 *  `ReceiptCitesWire` other than `checked` and `seen`. */
export interface ReceiptCiteWire {
  path: string;
  /** Exact resolved file recorded after restricted-path filtering. Older records omit it. */
  target?: string;
  line: number;
  end?: number;
  status: string;
}

/** S27 (`evidence::ReceiptVerification`, camelCase). Each flag is omitted
 *  (false) unless `claim` is `passed`, `done` or `fixed` and the receipt
 *  can tell; checks a subagent ran (a background one at any point), and
 *  every check of an agent that reports no tool calls (Antigravity), are
 *  unknown, so they never raise one. */
export interface ReceiptVerificationWire {
  /** `passed` | `done` | `fixed` | `none`. */
  claim: string;
  /** A pass claim with no recognised check after the run's last write
   *  (with no write, none in the run). */
  claimWithoutCheck?: boolean;
  /** A pass claim whose last recognised check failed. */
  claimAfterFail?: boolean;
}

/** One recorded outside effect (`run_effects::OutsideEffect`, camelCase). */
export interface OutsideEffectWire {
  /** `connector` | `hollow` | `shell` | `tool` | `mac`. */
  kind: string;
  label: string;
  tool: string;
  /** `done` | `failed` | `uncertain`. */
  outcome: string;
  /** Present when Hollow runs the call itself and can block a repeat. */
  key?: string | null;
  sequence: number;
}

/** Calls whose effects Hollow can't see (`run_effects::UnknownCalls`). */
export interface UnknownCallsWire {
  /** `shell` | `tool` | `helper`. */
  kind: string;
  label: string;
  count: number;
  examples?: string[];
}

/** `run_effects::OutsideEffects`, camelCase. */
export interface OutsideEffectsWire {
  recorded?: OutsideEffectWire[];
  unknown?: UnknownCallsWire[];
  moreRecorded?: number;
}

/** One pull request a turn merged or opened (overnight F8). */
export interface ReceiptPullRequest {
  number: number;
  repo?: string | null;
  action: 'merged' | 'opened' | string;
  url?: string | null;
}

/** A reply's canvas fences (`evidence::ReceiptCanvases`, camelCase): how
 *  many, and the UTF-8 bytes of their bodies, valid or not. */
export interface ReceiptCanvasesWire {
  count: number;
  bytes: number;
}

/** "1 canvas · 2.4 KB", "3 canvases · 18 KB": what a reply's canvases cost,
 *  for its footer and receipt. Null when it drew none. */
export function canvasesLine(canvases: ReceiptCanvasesWire | null | undefined): string | null {
  if (!canvases || !(canvases.count > 0)) return null;
  const kb = Math.max(0, canvases.bytes) / 1024;
  const size = kb < 1 ? `${Math.max(0, Math.round(canvases.bytes))} B` : kb < 10 ? `${kb.toFixed(1)} KB` : `${Math.round(kb)} KB`;
  return `${canvases.count} canvas${canvases.count === 1 ? '' : 'es'} · ${size}`;
}

/** One move a run made at a ceiling (`evidence::ReceiptSwitch`, camelCase):
 *  R1's quota failover to another Claude account, or a budget ceiling's
 *  move to another agent. `login` (2026-10-09) is the login override: a
 *  claude.ai connector kept the run on Claude Code's login (not a move).
 *  `retry` (overnight 9 A3): a provider outage's one later resume of the
 *  same session (not a move either). `rested` (overnight 9 A9): a large
 *  Main-agent chat started fresh after more than an hour away; its reason is
 *  "away 3 h at 714K". `from` and `to` are labels. */
export interface ReceiptSwitchWire {
  kind: 'account' | 'agent' | 'login' | 'retry' | 'rested';
  from: string;
  to: string;
  reason: string;
}

/** "Claude → Codex", the switch row's text (its reason shows beside it). */
export function switchLine(switched: Pick<ReceiptSwitchWire, 'from' | 'to'>): string {
  return `${switched.from} → ${switched.to}`;
}

/** One service a turn started (`evidence::ReceiptService`, camelCase). */
export interface ReceiptServiceWire {
  name: string;
  command: string;
  cwd: string;
  port?: number | null;
  kind: 'dev-server' | 'other';
  /** True when it stops with its chat (every mode but Bypass). */
  endsWithChat: boolean;
}

/** Requested → sent effort (`evidence::ReceiptEffort`, camelCase). */
export interface ReceiptEffortWire {
  /** A level, or `ultracode`. */
  requested: string;
  /** What the agent was given; null when its model takes no effort. */
  sent: string | null;
  /** Why they differ, when that matters (a CLI without ultracode). */
  note?: string | null;
}

/** "ultracode → xhigh", or just "high" when nothing was mapped; "high → none"
 *  when the model takes no effort. */
export function effortLine(effort: Pick<ReceiptEffortWire, 'requested' | 'sent'>): string {
  const sent = effort.sent ?? 'none';
  return sent === effort.requested ? sent : `${effort.requested} → ${sent}`;
}

/** Milliseconds per run phase (`evidence::PhaseTimings`, camelCase). */
export interface PhaseTimingsWire {
  /** The queue row's request → running. */
  queueWaitMs?: number;
  /** Admission's vault snapshot and manifest write. */
  baselineSnapshotMs?: number;
  /** Admitted → the agent spawned (preparation). */
  spawnMs?: number;
  /** Spawn → the first stdout or stderr line. */
  firstOutputMs?: number;
  /** Spawn → the first stdout line (agent effectiveness F11). */
  firstStdoutMs?: number;
  /** Spawn → the first model event: thinking, a tool call or text (OB08a). */
  firstModelMs?: number;
  /** Spawn → the first assistant text: the first useful output (F11). */
  firstTextMs?: number;
  /** A count, not a time: tool calls before the first text (OB08a). */
  toolsBeforeText?: number;
  /** Spawn → the agent's processes were gone. */
  turnMs?: number;
  /** The post-run repository capture (Paperclip PR 16). */
  repoCaptureMs?: number;
  /** The history diff and lease release. */
  finalizeDiffMs?: number;
  /** Building this receipt. */
  evidenceMs?: number;
}

/** One changed path in a repository (Rust `repo_diff::RepoFileChange`). */
export interface EvidenceRepoFileWire {
  path: string;
  changeType: 'create' | 'modify' | 'delete' | string;
  /** Porcelain XY, a committed row's raw letter, `clean` or `special`. */
  status: string;
  untracked: boolean;
  /** A submodule pointer: listed, never diffed. */
  gitlink: boolean;
  additions: number | null;
  deletions: number | null;
  binary: boolean;
  oversized: boolean;
  patchTruncated: boolean;
  /** A secret-named path: its content was never read or stored. */
  withheld: boolean;
  dirtyBefore: boolean;
  attribution: 'confirmed' | 'observed' | string;
  tool: string | null;
  /** Only in the sidecar (`RepoSidecar`), never in a receipt. */
  patch?: string;
}

/** One repository's changes (Rust `repo_diff::RepoChanges`). */
export interface EvidenceRepoChangesWire {
  root: string;
  label: string;
  target: 'cwd' | 'linkedProject' | string;
  status: 'captured' | 'unavailable' | string;
  error?: string | null;
  headBefore: string | null;
  headAfter: string | null;
  files: EvidenceRepoFileWire[];
  /** Every changed path, including rows past the listed ones. */
  totalFiles: number;
  truncated: boolean;
}

export interface EvidenceReceiptFileWire {
  path: string;
  changeType: string;
  attribution: string;
  tool: string | null;
  /** Lines added / removed (absent on older receipts, binary or large files). */
  additions?: number | null;
  deletions?: number | null;
  /** History holds no copy of one version, so it can't restore this file
   *  (merged plan slice 4). Absent when it can, and on older receipts. */
  notRecoverable?: boolean;
}

export interface EvidenceReceiptCheckWire {
  command: string;
  kind: string;
  outcome: string;
  sequence: number;
  excerpt: string | null;
}

export interface EvidenceReceiptFailureWire {
  label: string;
  sequence: number;
  excerpt: string;
}

/** Durable context the backend reports at the start of a run. */
export interface EvidenceRunContextWire {
  runId: string;
  /** Hollow's session key for this turn. The frontend echoes it back on the
   *  next send so a multi-turn conversation stays ONE session even when the
   *  harness never reports an id of its own. */
  sessionKey?: string | null;
  recall: EvidenceRecallItemWire[];
}

export interface EvidenceRecallItemWire {
  path: string;
  targetPath: string;
  subject: string;
  statement: string;
  score: number;
  /** Agent effectiveness F7 (`vault_learning::RecallItem.lifecycle`):
   *  `historical: superseded by …` for a replaced learning a historical
   *  request recalled, `current: supersedes …` for its replacement. */
  lifecycle?: string;
}

export interface ExecutionReceipt {
  status: ExecutionReceiptStatus;
  ordering: ReceiptOrdering;
  files: ReceiptFile[];
  checks: ReceiptCheck[];
  failures: ReceiptFailure[];
  /** Paths found by the filesystem diff without a matching structured write. */
  unattributedFileCount?: number;
  eventCount: number;
  /** Present only for durable backend receipts. */
  id?: string;
  runId?: string;
  source?: string;
  sourceId?: string | null;
  runStatus?: string;
  verificationState?: string;
  startedAt?: string;
  finishedAt?: string;
  historyRunId?: string;
  /** Recall injected for this exact run, joined by runId from its start event. */
  recall?: EvidenceRecallItemWire[];
  /** Paperclip PR 14a: the durable receipt's result kind, when it has one. */
  resultKind?: string;
  /** Repository changes (Paperclip PR 16), durable receipts only. */
  repoChanges?: EvidenceRepoChangesWire[];
  /** Sum of every repository's `totalFiles`. */
  repoFileCount?: number;
  /** Paperclip PR 15c: the durable receipt's phase timings, when it has any. */
  phaseTimings?: PhaseTimingsWire;
  /** Track U: requested → sent effort, when the turn asked for one. */
  effort?: ReceiptEffortWire;
  /** COO rebuild L7: the CFO's decision, for the footer's line. */
  cfo?: import('./generated/ipc').CfoDecision;
  /** Decision 6: moves the run made at a ceiling, in order. */
  switches?: ReceiptSwitchWire[];
  accountChoices?: string[];
  /** Slice 4: the durable receipt's `historyRecoverable`, when known. */
  historyRecoverable?: boolean;
  /** Canvas C3 wave 2 C4: the reply's canvas fences, when it drew any. */
  canvases?: ReceiptCanvasesWire;
  /** Overnight C3: when Hollow wakes this chat again, and why. */
  scheduledWake?: string;
  /** Overnight F8: the PRs it merged or opened; links only to github.com. */
  pullRequests?: ReceiptPullRequest[];
  /** Plan completion L2: what the run did beyond its files. */
  outsideEffects?: OutsideEffectsWire;
  /** R5: the final reply's cites, when it had any (`receiptCites.ts`). */
  cites?: ReceiptCitesWire;
  /** Repo features R1: how many times the agent compacted its context in
   *  the run (OB22's wire `compactions`); absent when none. */
  compactions?: number;
  /** Overnight A8: the reply says it is done, fixed, or that its checks
   *  pass, and the receipt doesn't back it (S27's flags); a failed last
   *  check wins over no check. */
  unbackedClaim?: 'afterFail' | 'noCheck';
  /** Which completion the reply claimed, when it claimed one. */
  claimKind?: 'passed' | 'done' | 'fixed';
}

/** Overnight A8: the verification row's chip text, the same words a parent
 *  chat reads on the child's line (`descendants::UNBACKED_*`). A done or
 *  fixed claim that nothing confirmed says "Not verified" (overhaul E3). */
export function unbackedClaimLabel(claim: ExecutionReceipt['unbackedClaim'], kind?: ExecutionReceipt['claimKind']): string | null {
  if (kind === 'done' || kind === 'fixed') {
    if (claim === 'afterFail' || claim === 'noCheck') return 'Not verified';
    return null;
  }
  if (claim === 'afterFail') return 'Says it passes · its last check failed';
  if (claim === 'noCheck') return 'Says it passes · no check after its last edit';
  return null;
}

/** P0-23: the reply footer's chip when the reply could look verified but
 *  isn't, in the Run record's own words, or null. A verified receipt, or one
 *  with no changes, never gets a status chip, though its cites still count.
 *  An unbacked pass claim wins. "Not verified" shows only for an unconfirmed
 *  done or fixed claim (overhaul E3): a change with no check, and no such
 *  claim, stays quiet. Then stale checks; then cites that don't resolve
 *  (past the end of the file, no such file, an unresolved name; a cite only
 *  outside the lines read, or read elsewhere, still points at something real). */
export function receiptAttentionLabel(receipt: Pick<ExecutionReceipt, 'status' | 'unbackedClaim' | 'claimKind' | 'cites'> | undefined): string | null {
  if (!receipt) return null;
  const claim = unbackedClaimLabel(receipt.unbackedClaim, receipt.claimKind);
  if (claim) return claim;
  if (receipt.status === 'stale') return receiptStatusLabel(receipt.status);
  const cites = receipt.cites;
  const broken = cites ? (cites.pastEnd ?? 0) + (cites.missing ?? 0) + (cites.unresolved ?? 0) : 0;
  if (broken > 0) return `${broken} ${broken === 1 ? 'cite doesn\'t' : 'cites don\'t'} resolve`;
  return null;
}

/** "Compacted · 1 time", "Compacted · 3 times" (repo features R1). */
export function compactionsLine(compactions: number | undefined): string | null {
  if (!compactions || !(compactions > 0)) return null;
  return `${compactions} time${compactions === 1 ? '' : 's'}`;
}

/** One rendered timing row: a measured phase and its formatted duration. */
export interface PhaseTimingRow {
  key: keyof PhaseTimingsWire;
  label: string;
  value: string;
}

const PHASE_TIMING_LABELS: ReadonlyArray<readonly [keyof PhaseTimingsWire, string]> = [
  ['queueWaitMs', 'Queue wait'],
  ['baselineSnapshotMs', 'Baseline snapshot'],
  ['spawnMs', 'Preparation'],
  ['firstOutputMs', 'First output (any line)'],
  ['firstStdoutMs', 'First program output'],
  ['firstModelMs', 'Model started'],
  ['firstTextMs', 'First text'],
  ['turnMs', 'Agent turn'],
  ['repoCaptureMs', 'Repository capture'],
  ['finalizeDiffMs', 'History diff'],
  ['evidenceMs', 'Receipt'],
];

/** The measured phases in run order; unmeasured ones are left out. */
export function phaseTimingRows(timings: PhaseTimingsWire | null | undefined): PhaseTimingRow[] {
  if (!timings) return [];
  const rows: PhaseTimingRow[] = [];
  for (const [key, label] of PHASE_TIMING_LABELS) {
    const ms = timings[key];
    if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) continue;
    rows.push({ key, label, value: formatPhaseDuration(ms) });
  }
  return rows;
}

/** `12ms`, `2.3s`, `2m 5s`, `1h 4m`. */
export function formatPhaseDuration(ms: number): string {
  return formatDuration(ms);
}

export const MAX_RECEIPT_EXCERPT_CHARS = 600;
export const MAX_RECEIPT_COMMAND_CHARS = 320;

/** Whether a receipt file is credited to its run: anything but `shared`
 *  (another chat in the same workspace folder changed it, chat concurrency
 *  F1, 2026-10-20). Chips, counts and totals use only these; the expanded
 *  receipt still lists shared rows, labelled. */
export function isOwnReceiptFile(file: { attribution?: string }): boolean {
  return file.attribution !== 'shared';
}

/** The plain label a receipt file without a tool shows (chat concurrency
 *  F1, 2026-10-20: `shared` is another chat's file in the same folder). */
export function receiptFileOrigin(attribution: string): string {
  if (attribution === 'observed') return 'filesystem observation';
  if (attribution === 'shared') return 'changed by another chat';
  return 'tool unavailable';
}

/**
 * Adapt the backend's durable receipt into the shared presentation model.
 *
 * Recall is deliberately joined only when its start event names the exact
 * same run. This prevents a late event from attributing context from one run
 * to another, and avoids positional receipt-to-turn guesses on reload.
 */
function receiptClaimKind(claim: string | undefined): ExecutionReceipt['claimKind'] {
  if (claim === 'passed' || claim === 'done' || claim === 'fixed') return claim;
  return undefined;
}

export function executionReceiptFromWire(
  wire: EvidenceReceiptWire,
  runContext?: EvidenceRunContextWire | null,
): ExecutionReceipt {
  const files = wire.files.map((file, index): ReceiptFile => ({
    path: file.path,
    tool: file.tool ?? receiptFileOrigin(file.attribution),
    sequence: index,
    changeType: file.changeType,
    attribution: file.attribution,
    ...(typeof file.additions === 'number' ? { additions: file.additions } : {}),
    ...(typeof file.deletions === 'number' ? { deletions: file.deletions } : {}),
    ...(file.notRecoverable ? { notRecoverable: true } : {}),
  }));
  const checks = wire.checks.map((check): ReceiptCheck => ({
    // Already bounded and shell-sanitized by the recorder. Copy must retain
    // those exact bytes, including its literal truncation marker.
    command: check.command,
    kind: receiptCheckKindFromWire(check.kind),
    outcome: receiptCheckOutcomeFromWire(check.outcome),
    sequence: finiteSequence(check.sequence),
    excerpt: check.excerpt ? capReceiptExcerpt(check.excerpt) : undefined,
  }));
  const failures = wire.failures.map((failure): ReceiptFailure => ({
    tool: failure.label || 'run',
    sequence: finiteSequence(failure.sequence),
    excerpt: capReceiptExcerpt(failure.excerpt || 'Operation failed.'),
  }));
  const repoChanges = (wire.repoChanges ?? []).map(repo => ({
    ...repo,
    files: repo.files.map(file => ({ ...file })),
  }));
  const repoFileCount = repoChanges.reduce((sum, repo) => sum + (repo.totalFiles || 0), 0);
  const unattributedFileCount = files.filter(file => file.attribution === 'observed').length
    + repoChanges.reduce(
      (sum, repo) => sum + repo.files.filter(file => file.attribution !== 'confirmed').length,
      0,
    );
  const ordering: ReceiptOrdering = wire.verificationState === 'unknownOrder' ? 'unknown' : 'known';
  const status = receiptStatusFromWire(wire);
  const recall = runContext?.runId === wire.runId && runContext.recall.length > 0
    ? runContext.recall.map(item => ({ ...item }))
    : undefined;
  const claimKind = receiptClaimKind(wire.verification?.claim);

  return {
    status,
    ordering,
    files,
    checks,
    failures,
    unattributedFileCount,
    eventCount: files.length + checks.length + failures.length,
    id: wire.id,
    runId: wire.runId,
    source: wire.source,
    sourceId: wire.sourceId,
    runStatus: wire.status,
    verificationState: wire.verificationState,
    startedAt: wire.startedAt,
    finishedAt: wire.finishedAt,
    historyRunId: wire.historyRunId,
    recall,
    ...(wire.resultKind ? { resultKind: wire.resultKind } : {}),
    ...(repoChanges.length > 0 ? { repoChanges, repoFileCount } : {}),
    ...(wire.phaseTimings ? { phaseTimings: { ...wire.phaseTimings } } : {}),
    ...(wire.effort ? { effort: { ...wire.effort } } : {}),
    ...(wire.cfo ? { cfo: { ...wire.cfo } } : {}),
    ...(wire.accountChoices?.length ? { accountChoices: wire.accountChoices.map(capReceiptExcerpt) } : {}),
    ...(wire.switches && wire.switches.length > 0
      ? { switches: wire.switches.map(switched => ({ ...switched })) }
      : {}),
    ...(typeof wire.historyRecoverable === 'boolean' ? { historyRecoverable: wire.historyRecoverable } : {}),
    ...(wire.canvases && wire.canvases.count > 0 ? { canvases: { count: wire.canvases.count, bytes: wire.canvases.bytes } } : {}),
    ...(wire.scheduledWake && wire.scheduledWake.trim() ? { scheduledWake: capReceiptExcerpt(wire.scheduledWake.trim()) } : {}),
    ...(wire.pullRequests && wire.pullRequests.length > 0 ? { pullRequests: receiptPullRequests(wire.pullRequests) } : {}),
    // Passed through: the views bound what they list (plan completion L2).
    ...(wire.outsideEffects ? { outsideEffects: wire.outsideEffects } : {}),
    // R5: passed through; the lazy Cites row bounds and words it.
    ...(wire.cites && wire.cites.checked > 0 ? { cites: wire.cites } : {}),
    ...(typeof wire.compactions === 'number' && wire.compactions > 0 ? { compactions: wire.compactions } : {}),
    ...(claimKind ? { claimKind } : {}),
    ...(wire.verification?.claimAfterFail
      ? { unbackedClaim: 'afterFail' as const }
      : wire.verification?.claimWithoutCheck ? { unbackedClaim: 'noCheck' as const } : {}),
  };
}

/** A receipt's PRs, safe to render: a link only to `https://github.com/`,
 *  at most 20 rows. */
export function receiptPullRequests(rows: readonly ReceiptPullRequest[]): ReceiptPullRequest[] {
  return rows.slice(0, 20).map(p => ({
    number: p.number,
    repo: p.repo ?? null,
    action: p.action === 'merged' ? 'merged' : 'opened',
    url: p.url && p.url.startsWith('https://github.com/') ? p.url : null,
  }));
}

/** `Merged #453` / `Opened #12 in o/r`. */
export function pullRequestLabel(p: ReceiptPullRequest): string {
  return `${p.action === 'merged' ? 'Merged' : 'Opened'} #${p.number}${p.repo ? ` in ${p.repo}` : ''}`;
}

/**
 * What a receipt says about undoing the run (merged plan slice 4, decision
 * 5): null unless History can't restore some file, so a receipt never claims
 * more than History holds.
 */
export function receiptRecoveryNote(receipt: Pick<ExecutionReceipt, 'files' | 'historyRecoverable'>): string | null {
  const count = receipt.files.filter(file => file.notRecoverable).length;
  if (count === 0 && receipt.historyRecoverable !== false) return null;
  if (count === 0) return 'Not fully recoverable: History can’t restore every change in this run.';
  return `Not fully recoverable: History has no copy of ${count === 1 ? 'one changed file' : `${count} changed files`}, so it can’t restore ${count === 1 ? 'it' : 'them'}.`;
}

/**
 * Whether a durable receipt recorded repository changes (Paperclip PR 16): a
 * changed path, or a repository whose changes could not be recorded. Such a
 * run is never "no changes", whatever an older backend's state says.
 */
export function receiptHasRepoEvidence(wire: Pick<EvidenceReceiptWire, 'repoChanges'>): boolean {
  return (wire.repoChanges ?? []).some(repo => repo.status !== 'captured' || repo.totalFiles > 0);
}

export function receiptStatusLabel(status: ExecutionReceiptStatus): string {
  switch (status) {
    case 'verified': return 'Verified';
    case 'failed': return 'Failed';
    case 'stale': return 'Checks out of date';
    case 'unverified': return 'Not verified';
    case 'no_changes': return 'No changes';
  }
}

/** Human-readable copy that avoids implying more certainty than the events support. */
export function executionReceiptStateText(receipt: ExecutionReceipt): string {
  switch (receipt.status) {
    case 'verified':
      return 'The final observed changes were followed by successful observed checks.';
    case 'failed':
      if (
        !receipt.files.some(isOwnReceiptFile)
        && (receipt.repoFileCount ?? 0) === 0
        && !hasUnavailableRepo(receipt)
        && receipt.runStatus
        && receipt.runStatus !== 'success'
      ) {
        return `The run finished ${receipt.runStatus}; no completed file changes were observed.`;
      }
      return 'An observed check or operation failed; this run is not verified.';
    case 'stale':
      return 'Checks ran before the final observed change, but not afterward.';
    case 'no_changes':
      return 'No completed file changes were observed.';
    case 'unverified': {
      if (hasUnavailableRepo(receipt)) {
        return 'Repository changes for this run could not be fully recorded, so it cannot be marked verified or unchanged.';
      }
      const unknownOrder = receipt.ordering === 'unknown';
      const unattributed = (receipt.unattributedFileCount ?? 0) > 0;
      if (unknownOrder && unattributed) {
        return 'Changes were observed from filesystem history, but their tool attribution or event order was incomplete; verification cannot be established.';
      }
      if (unknownOrder) {
        return 'Changes and checks were observed, but their order was unavailable; verification cannot be established.';
      }
      return 'Changes were observed without a successful check afterward.';
    }
  }
}

function hasUnavailableRepo(receipt: ExecutionReceipt): boolean {
  return (receipt.repoChanges ?? []).some(repo => repo.status !== 'captured');
}

function receiptStatusFromWire(wire: EvidenceReceiptWire): ExecutionReceiptStatus {
  // A process-level failure remains a failed run even when it changed no files.
  if (wire.status !== 'success') return 'failed';
  // A repo-only run is never "no changes" (Paperclip PR 16).
  if (wire.verificationState === 'noChanges' && receiptHasRepoEvidence(wire)) return 'unverified';
  switch (wire.verificationState) {
    case 'verifiedAfterChange': return 'verified';
    case 'failedAfterChange': return 'failed';
    case 'staleAfterChange': return 'stale';
    case 'notVerified':
    case 'unknownOrder': return 'unverified';
    case 'noChanges': return 'no_changes';
    default: return 'unverified';
  }
}

function receiptCheckKindFromWire(value: string): ReceiptCheckKind {
  switch (value) {
    case 'test':
    case 'build':
    case 'typecheck':
    case 'lint':
    case 'check':
    case 'gate': return value;
    default: return 'check';
  }
}

function receiptCheckOutcomeFromWire(value: string): ReceiptCheckOutcome {
  switch (value) {
    case 'passed':
    case 'failed':
    case 'unknown': return value;
    default: return 'unknown';
  }
}

function finiteSequence(value: number): number {
  return Number.isFinite(value) ? value : 0;
}

export function capReceiptExcerpt(value: string): string {
  return capText(value.replace(/\r\n?/g, '\n').replace(/\0/g, '�').trim(), MAX_RECEIPT_EXCERPT_CHARS);
}

function capText(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, Math.max(0, max - 1))}…`;
}
