/** One welcome page, mounted outside App in its own lazy root. Optional
 *  context, import and source panels retain their owners. The existing setup
 *  record stays compatible; Close leaves it open, starting finishes it.
 *  Settings About and the existing Sources route reopen the page. */
import { For, Show, Suspense, createSignal, lazy, onCleanup, onMount } from 'solid-js';
import { api, type ImportCommitResult } from '../api';
import { SettingsSheet } from '../settings/SettingsSheet';
import { Skeleton, ErrorText } from './ui';
import { AgentMark } from './AgentMark';
import { firstRunSetStep, type SetupStatus } from '../firstRunApi';
import {
  SHEET_TITLE, ACP_REGISTRY_URL, agentLine, finishWelcome, welcomeAgents, welcomeReady, welcomeCapabilityLines,
  importedFromBatches, startSummary, workspaceNameProblem, type WelcomeAgent,
} from '../firstRun';
import { contextScanStatus, startContextScan } from '../contextReviewApi';
import { scanText } from '../contextReview';
import { errMsg, openExternal } from '../utils';
import '../styles/firstRun.css';
import '../styles/connectors-library.css';

const ContextLayersPanel = lazy(() => import('../settings/ContextLayersPanel').then(m => ({ default: m.ContextLayersPanel })));
const SourcesPanel = lazy(() => import('../settings/SourcesPanel').then(m => ({ default: m.SourcesPanel })));
const TaskTrackerPanel = lazy(() => import('../settings/TaskTrackerPanel').then(m => ({ default: m.TaskTrackerPanel })));
const ImportConversations = lazy(() => import('./ImportConversations'));
const ContextReviewCard = lazy(() => import('./ContextReviewCard'));
const WelcomePhone = lazy(() => import('../settings/CompanionDevicesPanel').then(m => ({ default: m.WelcomePhone })));

/** How often a running context scan is asked for its progress. */
const SCAN_POLL_MS = 700;

function FindStep(props: { onImported: (result: ImportCommitResult) => void }) {
  const [scanId, setScanId] = createSignal<string | null>(null);
  const [scan, setScan] = createSignal<{ state: 'idle' | 'scanning' | 'ready' | 'cancelled' | 'failed'; found: number; error?: string | null }>({ state: 'idle', found: 0 });
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => { alive = false; if (timer) clearTimeout(timer); });

  async function poll(id: string) {
    try {
      const status = await contextScanStatus(id);
      if (!alive) return;
      if (!status || status.state === 'scanning') { timer = setTimeout(() => void poll(id), SCAN_POLL_MS); return; }
      setScan({ state: status.state, found: status.found, error: status.error });
      if (status.state === 'ready') setScanId(id);
    } catch (e) { if (alive) setScan({ state: 'failed', found: 0, error: errMsg(e) }); }
  }
  async function start() {
    if (scan().state === 'scanning') return;
    setScanId(null); setScan({ state: 'scanning', found: 0 });
    try { const started = await startContextScan(); if (alive) void poll(started.scanId); }
    catch (e) { setScan({ state: 'failed', found: 0, error: errMsg(e) }); }
  }

  return <div class="first-run-find">
    <h4>Conversations</h4>
    <Suspense fallback={<Skeleton label="Loading import" rows={2} />}><ImportConversations onImported={r => props.onImported(r)} /></Suspense>
    <h4>Instruction and memory files</h4>
    <p class="settings-row-v">CLAUDE.md, AGENTS.md, rules and memory folders. Nothing is copied until you accept one.</p>
    <div class="work-actions">
      <button type="button" class="btn small" disabled={scan().state === 'scanning'} onClick={() => void start()}>
        {scan().state === 'scanning' ? 'Scanning' : 'Scan for instruction and memory files'}
      </button>
    </div>
    <Show when={scan().state !== 'idle'}><p class="settings-row-v" role="status">{scanText(scan().state, scan().found, scan().error)}</p></Show>
    <Show when={scanId()} keyed>{id =>
      <Suspense fallback={<Skeleton label="Loading found context" rows={2} />}><ContextReviewCard scanId={id} showScan={false} /></Suspense>}
    </Show>
  </div>;
}

function SourcesStep() {
  const [tracker, setTracker] = createSignal(false);
  return <div class="first-run-sources">
    <Suspense fallback={<Skeleton label="Loading sources" rows={3} />}><SourcesPanel /></Suspense>
    <div class="work-actions">
      <button type="button" class="btn small" aria-expanded={tracker()} onClick={() => setTracker(v => !v)}>
        {tracker() ? 'Hide the task tracker' : 'Connect a task tracker'}
      </button>
    </div>
    <Show when={tracker()}>
      <Suspense fallback={<Skeleton label="Loading the task tracker" rows={2} />}><TaskTrackerPanel /></Suspense>
    </Show>
  </div>;
}

function AgentsStep(props: { onOpenAccounts: () => void }) {
  const [rows, setRows] = createSignal<WelcomeAgent[] | null>(null);
  const [error, setError] = createSignal('');
  const [checking, setChecking] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });
  async function refresh() {
    if (checking()) return;
    setChecking(true); setError('');
    try {
      const [descriptors, clis, accounts] = await Promise.all([api.agentDescriptors(), api.agentCliStatus(false), api.providerAccounts()]);
      if (alive) setRows(welcomeAgents(descriptors, clis, accounts));
    } catch (e) { if (alive) { setRows(null); setError(errMsg(e)); } }
    finally { if (alive) setChecking(false); }
  }
  onMount(() => void refresh());
  return <div class="first-run-agents">
    <Show when={rows()} fallback={<Show when={!error()}><p class="settings-loading">Checking agents…</p></Show>}>
      {list => <>
        <p class="settings-row-v" role="status">{list().filter(welcomeReady).length} of {list().length} agents ready</p>
        <ul class="first-run-agent-list" aria-label="Agents on this Mac">
          <For each={list()}>{row =>
            <li class="first-run-agent" data-agent={row.agent}>
              <AgentMark agent={row.agent} />
              <span class="first-run-agent-name">{row.label}</span>
              <Show when={welcomeReady(row)}><span class="claude-acct-status" data-status="ok">Ready</span></Show>
              <details class="first-run-agent-details">
                <summary aria-label={`Details for ${row.label}`}>Details</summary>
                <p>{agentLine(row)}{row.installed && row.signedIn === null ? '. Sign-in has not been verified.' : ''}</p>
                <ul><For each={welcomeCapabilityLines(row)}>{line => <li>{line}</li>}</For></ul>
                <button type="button" class="btn small" onClick={props.onOpenAccounts}>Manage in Accounts</button>
              </details>
            </li>}
          </For>
        </ul>
      </>}
    </Show>
    <Show when={error()}><ErrorText class="work-error" text={error()} /></Show>
    <div class="work-actions">
      <button type="button" class="btn small" onClick={() => openExternal(ACP_REGISTRY_URL)}>Add more agents</button>
      <button type="button" class="btn small ghost" onClick={props.onOpenAccounts}>Open Accounts</button>
      <button type="button" class="btn small ghost" disabled={checking()} onClick={() => void refresh()}>Check again</button>
    </div>
    <p class="settings-row-v">Browse the ACP registry. Hollow lists its supported agents above; manage installation and sign-in in Accounts.</p>
  </div>;
}

function StartStep(props: { imported: number | null; onWorkspace: (name: string) => Promise<void>; onChats: () => Promise<void> }) {
  const [name, setName] = createSignal('');
  const [suggestions, setSuggestions] = createSignal<number | null>(null);
  /** The committed batches' count, read when this session imported nothing
   *  (after a quit and resume); null while unknown, so nothing false is said. */
  const [committed, setCommitted] = createSignal<number | null>(null);
  onMount(async () => {
    if (props.imported) return;
    try { setCommitted(importedFromBatches((await api.sessionImportStatus()).batches)); } catch { setCommitted(null); }
  });
  const imported = () => (props.imported ? props.imported : committed() ?? props.imported);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  onMount(async () => {
    try { setSuggestions((await api.listLearningProposals()).filter(p => p.status === 'pending').length); } catch { setSuggestions(null); }
  });
  const problem = () => workspaceNameProblem(name());
  async function run(action: () => Promise<void>) {
    if (busy()) return;
    setBusy(true); setError('');
    try { await action(); } catch (e) { setError(errMsg(e)); } finally { setBusy(false); }
  }
  return <div class="first-run-start">
    <ul class="first-run-summary">
      <For each={startSummary(imported(), suggestions())}>{line => <li>{line}</li>}</For>
    </ul>
    <label class="first-run-field">
      <span>First workspace (optional)</span>
      <input type="text" value={name()} placeholder="For example, Pricing study" maxLength={120} onInput={e => setName(e.currentTarget.value)} />
    </label>
    <Show when={problem()}>{p => <p class="settings-row-v" role="note">{p()}</p>}</Show>
    <div class="work-actions">
      <button type="button" class="btn small" disabled={busy() || !name().trim() || !!problem()} onClick={() => void run(() => props.onWorkspace(name().trim()))}>Create workspace</button>
      <button type="button" class="btn small ghost" disabled={busy()} onClick={() => void run(props.onChats)}>Open Chats</button>
    </div>
    <Show when={error()}><ErrorText class="work-error" text={error()} /></Show>
  </div>;
}

export function FirstRunSheet(props: { status: SetupStatus; onClose: () => void }) {
  const [status, setStatus] = createSignal<SetupStatus>(props.status);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [contextOpen, setContextOpen] = createSignal(false);
  const [sourcesOpen, setSourcesOpen] = createSignal(false);
  const [findOpen, setFindOpen] = createSignal(false);
  const [phoneOpen, setPhoneOpen] = createSignal(false);
  /** Conversations imported in this session's Find step; null when none was. */
  const [imported, setImported] = createSignal<number | null>(null);
  /** A failed action leaves setup open; each saved completion is retained for retry. */
  async function finishWith(action: () => Promise<void>) {
    if (busy()) return;
    setBusy(true); setError('');
    try {
      await action();
      await finishWelcome(status(), firstRunSetStep, setStatus);
      props.onClose();
    } catch (e) { setError(errMsg(e)); }
    finally { setBusy(false); }
  }

  const openAccounts = async () => {
    const { openSettingsFromOutside } = await import('../store/runtime');
    openSettingsFromOutside('accounts');
    props.onClose();
  };
  /** The workspace Start made, so a retry after a failed open opens it
   *  instead of making a second one. */
  let createdId: string | null = null;
  const openWorkspace = (name: string) => finishWith(async () => {
    createdId ??= (await api.createWorkspace(name, name)).id;
    const ws = await import('../store/workspaces');
    await ws.refreshWorkspaces();
    if (!await ws.selectWorkspaceOpened(createdId)) throw new Error('The workspace could not open. Your setup is still here.');
  });
  const openChats = () => finishWith(async () => {
    if (!await (await import('../store/workspaces')).startQuickChatForFiles()) throw new Error('Chats could not open. Your setup is still here.');
  });

  return <SettingsSheet id="first-run-sheet" title={SHEET_TITLE} onClose={props.onClose} locked={busy}>
    <div class="first-run">
      <p class="settings-row-v">Your projects, conversations and notes, together on this Mac.</p>
      <fieldset class="first-run-body" disabled={busy()}>
        <legend class="sr-only">Welcome setup</legend>
        <h3 class="first-run-title">Your agents</h3>
        <AgentsStep onOpenAccounts={() => void openAccounts().catch(e => setError(errMsg(e)))} />
        <details class="first-run-section" onToggle={e => { if (e.currentTarget.open) setFindOpen(true); }}>
          <summary>Import conversations and context</summary>
          <Show when={findOpen()}><FindStep onImported={r => setImported(prev => (prev ?? 0) + r.imported + r.updated + r.newVersions)} /></Show>
        </details>
        <details class="first-run-section" onToggle={e => { if (e.currentTarget.open) setContextOpen(true); }}>
          <summary>Connect your context</summary>
          <Show when={contextOpen()}><Suspense fallback={<Skeleton label="Loading context layers" rows={3} />}><ContextLayersPanel /></Suspense></Show>
        </details>
        <details class="first-run-section" onToggle={e => { if (e.currentTarget.open) setSourcesOpen(true); }}>
          <summary>Where your work lives</summary>
          <Show when={sourcesOpen()}><SourcesStep /></Show>
        </details>
        <details class="first-run-section" onToggle={e => { if (e.currentTarget.open) setPhoneOpen(true); }}>
          <summary>Set up your iPhone</summary>
          <Show when={phoneOpen()}><Suspense fallback={<Skeleton label="Loading iPhone setup" rows={2} />}><WelcomePhone /></Suspense></Show>
        </details>
        <h3 class="first-run-title">Start</h3>
        <StartStep imported={imported()} onWorkspace={openWorkspace} onChats={openChats} />
      </fieldset>
      <Show when={error()}><ErrorText class="work-error" text={error()} /></Show>
      <div class="first-run-nav" role="group" aria-label="Setup navigation">
        <button type="button" class="btn ghost" disabled={busy()} onClick={() => void finishWith(async () => {})}>Start without an agent</button>
        <p class="settings-row-v">You can add an agent later in Settings, Accounts.</p>
      </div>
    </div>
  </SettingsSheet>;
}

export default FirstRunSheet;
