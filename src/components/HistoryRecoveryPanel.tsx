import { coverageLabel, isChangesUnknown, isSharedRestoreConflict, overlapLine, restorePreview, runFilesLine, runOutcome, runRecoverable, runTitle, sharedChanges, sideCoverage, uncapturedReason } from '../historyRun';
import { workspaces } from '../store/workspaces';
import { For, Show, Suspense, createEffect, createMemo, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import { flashFooter, requestOpenVaultNote, setActivePage, showConfirm } from '../store';
import { protectedFlags, runFlagLine, sharedDocFlags } from '../workflowPermissions';
import { preserveDuringRefetch } from '../stableResource';
import { ResourceReadError } from './ResourceReadError';
import { Skeleton } from './ui';

type HistoryRun = Awaited<ReturnType<typeof api.listHistoryRuns>>[number];
type HistoryChange = HistoryRun['changes'][number];

function fmtBytes(value: number): string {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

function fmtWhen(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function nonEmpty<T>(items: T[]): T[] | undefined {
  return items.length > 0 ? items : undefined;
}

function notionRestoreWarning(changes: readonly HistoryChange[]): string {
  const count = changes.filter(change => change.notionMirror).length;
  return count === 0
    ? ''
    : `\n\n${count} ${count === 1 ? 'file was' : 'files were'} imported from Notion. Restoring changes only your Obsidian vault; it does not change the source page.`;
}

/** Run-scoped recovery UI. Restores always delegate conflict detection and
 * reversible snapshotting to the Rust history store. */
export function HistoryRecoveryPanel(props: { notePath?: string; runId?: string }) {
  // A new note is a new owner; retained data must never cross note scopes.
  const scope = createMemo(() => ({ notePath: props.notePath }), undefined, {
    equals: (previous, next) => previous.notePath === next.notePath,
  });
  return <Show when={scope()} keyed>{current => <Suspense fallback={<Skeleton label="Reading history" rows={3} />}><HistoryRecoveryContent notePath={current.notePath} runId={props.runId} /></Suspense>}</Show>;
}

function HistoryRecoveryContent(props: { notePath?: string; runId?: string }) {
  const [selectedId, setSelectedId] = createSignal<string | null>(props.runId ?? null);
  const [preview, setPreview] = createSignal<{ runId: string; path: string; before: string | null; after: string | null } | null>(null);
  const [busy, setBusy] = createSignal<string | null>(null);
  const [runRead, { refetch: refetchRuns }] = createResource(
    () => props.notePath ?? '',
    path => path ? api.listNoteHistory(path, 200) : api.listHistoryRuns(200),
  );
  const [storageRead, { refetch: refetchStorage }] = createResource(() => api.historyStorageStatus());
  const runs = preserveDuringRefetch(runRead);
  const storage = preserveDuringRefetch(storageRead);
  const selected = createMemo(() => selectedId()
    ? (runs() ?? []).find(run => run.id === selectedId()) ?? null
    : (runs() ?? [])[0] ?? null);

  createEffect(() => {
    void props.notePath;
    setSelectedId(props.runId ?? null);
    setPreview(null);
  });

  async function loadPreview(run: HistoryRun, change: HistoryChange) {
    setBusy(`${run.id}:${change.vaultPath}:preview`);
    try {
      const [before, after] = await Promise.all([
        api.readHistoryVersion(run.id, change.vaultPath, 'before'),
        api.readHistoryVersion(run.id, change.vaultPath, 'after'),
      ]);
      setPreview({ runId: run.id, path: change.vaultPath, before, after });
    } catch (error: any) {
      flashFooter(`× preview unavailable: ${error?.message || error}`, 3200);
    } finally {
      setBusy(null);
    }
  }

  async function restoreChange(run: HistoryRun, change: HistoryChange, force = false, sharedForce = false) {
    const label = `${run.id}:${change.vaultPath}:restore`;
    if (busy()) return;
    const shared = (change.alsoChangedBy ?? []).length > 0;
    const ok = await showConfirm({
      title: force ? 'Force restore this file?' : 'Restore this file?',
      body: force && change.unverified
        ? `${change.vaultPath}\n\nChanges unknown: the run that listed this file ended with Hollow's process, so the list may include edits you made while Hollow was down.${sharedForce ? ' Another conversation in this workspace also changed it.' : ''} Hollow will snapshot the current file first, then replace it with the pre-run version.${notionRestoreWarning([change])}`
        : force && sharedForce
        ? `${change.vaultPath}\n\nAnother conversation in this workspace also changed this file. Hollow will snapshot the current file first, then replace it with the pre-run version without checking whether it changed since this run.${notionRestoreWarning([change])}`
        : force
        ? `${change.vaultPath}\n\nThe current file no longer matches the saved after-version. Hollow will snapshot it first, then replace it with the pre-run version.${notionRestoreWarning([change])}`
        : `${change.vaultPath}\n\nHollow will restore the pre-run version only if the current file still matches this run. The restore is recorded as a new reversible history run.${shared ? '\n\nAnother conversation in this workspace also changed this file while this run was going; Hollow will ask again before undoing its work.' : ''}${notionRestoreWarning([change])}`,
      confirmLabel: force ? 'Force restore' : 'Restore',
      danger: force,
    });
    if (!ok) return;
    setBusy(label);
    try {
      await api.restoreHistoryChange(run.id, change.vaultPath, force);
      await Promise.all([refetchRuns(), refetchStorage()]);
      flashFooter('File restored; the restore can be undone from History', 2800);
    } catch (error: any) {
      const message = error?.message || String(error);
      if (!force && /conflict|after-version|no longer matches/i.test(message)) {
        setBusy(null);
        const retry = await showConfirm({
          title: isSharedRestoreConflict(message) ? 'Another conversation also changed this file' : isChangesUnknown(message) ? 'Changes unknown' : 'File changed since this run',
          body: isSharedRestoreConflict(message)
            ? `${change.vaultPath}\n\nAnother conversation in this workspace ran at the same time and changed this file too. History can’t tell whose edit it was, so restoring it could undo that conversation’s work.\n\nForce restore snapshots the current version first but replaces it.`
            : isChangesUnknown(message)
            ? `${change.vaultPath}\n\nThis run ended with Hollow's process, so its list of changes may include edits you made while Hollow was down. Restoring could undo them.\n\nForce restore snapshots the current version first but replaces it.`
            : `${change.vaultPath}\n\n${message}\n\nForce restore snapshots the current version first but replaces it.`,
          confirmLabel: 'Review force restore',
          danger: true,
        });
        if (retry) await restoreChange(run, change, true, isSharedRestoreConflict(message));
        return;
      }
      flashFooter(`× restore failed: ${message}`, 3500);
    } finally {
      setBusy(null);
    }
  }

  async function restoreRun(run: HistoryRun, force = false) {
    if (busy() || run.changes.length === 0) return;
    const ok = await showConfirm({
      title: force ? 'Force restore this entire run?' : 'Restore this entire run?',
      body: force
        ? `${run.changes.length} files will be returned to their pre-run versions. Current content is snapshotted first, but every file will be replaced.${notionRestoreWarning(run.changes)}`
        : `Hollow will preflight all ${run.changes.length} files before writing anything. If any current file differs, the entire restore is cancelled with no partial writes.${notionRestoreWarning(run.changes)}`,
      confirmLabel: force ? 'Force restore run' : 'Restore run',
      danger: force,
    });
    if (!ok) return;
    setBusy(`${run.id}:run`);
    try {
      await api.restoreHistoryRun(run.id, force);
      await Promise.all([refetchRuns(), refetchStorage()]);
      flashFooter(`${run.changes.length} changes restored as one reversible run`, 2800);
    } catch (error: any) {
      const message = error?.message || String(error);
      if (isSharedRestoreConflict(message)) {
        flashFooter('× another conversation also changed some of these files; restore them one at a time', 4200);
        return;
      }
      if (!force && isChangesUnknown(message)) {
        setBusy(null);
        const retry = await showConfirm({
          title: 'Changes unknown',
          body: `This run ended with Hollow's process, so its list of changes may include edits you made while Hollow was down. Restoring the whole run could undo them.\n\nForce restore snapshots every current file first but replaces them.`,
          confirmLabel: 'Review force restore',
          danger: true,
        });
        if (retry) await restoreRun(run, true);
        return;
      }
      flashFooter(`× run restore failed: ${message}`, 3500);
    } finally {
      setBusy(null);
    }
  }

  async function clearHistory() {
    const ok = await showConfirm({
      title: 'Clear automated-write history?',
      body: 'This permanently removes history manifests and snapshots. Vault notes are not touched, but past automated changes can no longer be previewed or restored.',
      confirmLabel: 'Clear history',
      danger: true,
    });
    if (!ok) return;
    setBusy('clear');
    try {
      await api.clearHistory();
      setSelectedId(null); setPreview(null);
      await Promise.all([refetchRuns(), refetchStorage()]);
      flashFooter('Automated-write history cleared', 2400);
    } catch (error: any) {
      flashFooter(`× clear failed: ${error?.message || error}`, 3500);
    } finally { setBusy(null); }
  }

  function openFile(path: string) {
    setActivePage('vault');
    requestOpenVaultNote(path);
  }

  return (
    <div class="history-recovery">
      <ResourceReadError label="history" read={runs} retry={refetchRuns} />
      <ResourceReadError label="history storage" read={storage} retry={refetchStorage} />
      <Show when={storage()}>
        {status => (
          <details class="history-storage"><summary>History storage · {status().runCount} runs kept</summary><div class="history-storage-strip">
            <span><strong>{status().runCount}</strong> runs</span>
            <span><strong>{status().objectCount}</strong> deduplicated versions</span>
            <span><strong>{fmtBytes(status().totalBytes)}</strong> used</span>
            <span>keeps {status().maxRuns} runs / {status().maxAgeDays} days</span>
            <Show when={status().activeRunCount > 0}><span class="history-active-warning">{status().activeRunCount} active; clear locked</span></Show>
            <button class="btn ghost sm" title={status().activeRunCount > 0 ? 'History cannot be cleared while an automated run is active.' : undefined} disabled={busy() === 'clear' || status().runCount === 0 || status().activeRunCount > 0} onClick={clearHistory}>Clear history</button>
          </div></details>
        )}
      </Show>
      <div class="history-layout">
        <div class="history-run-list" role="listbox" aria-label="Automated-write runs" data-settings-control="Automated-write runs">
          <Show when={(runs() ?? []).length > 0} fallback={<Show when={!runs.error || runs() !== undefined}><div class="settings-empty">No automated writes have been recorded yet.</div></Show>}>
            <For each={runs() ?? []}>
              {run => (
                <button
                  class="history-run-row"
                  classList={{ active: selected()?.id === run.id }}
                  aria-selected={selected()?.id === run.id}
                  onClick={() => { setSelectedId(run.id); setPreview(null); }}
                >
                  <span class="history-run-source">{runTitle(run, id => workspaces().find(w => w.id === id)?.title)}</span>
                  <span class="history-run-meta">{fmtWhen(run.startedAt)} · {runFilesLine(run)}</span>
                  <span class="settings-state-chip" data-state={runOutcome(run.status).tone}>{runOutcome(run.status).label}</span>
                </button>
              )}
            </For>
          </Show>
        </div>
        <div class="history-run-detail">
          <Show when={selected()} fallback={<Show when={!runs.error || runs() !== undefined}><div class="settings-empty">{selectedId() ? 'The selected run is not available in retained history. Choose a listed run to inspect it.' : 'Select a run to inspect it.'}</div></Show>}>
            {run => (
              <>
                <div class="history-detail-head">
                  <div><strong>{runTitle(run(), id => workspaces().find(w => w.id === id)?.title)}</strong>
                    <div class="settings-row-v">{fmtWhen(run().startedAt)} · {runOutcome(run().status).label} · {run().changes.length} {run().changes.length === 1 ? 'file' : 'files'} changed{run().scope ? '; its workspace folder and what it changed through Hollow are recorded here, repository changes are in the run’s record' : ''}</div>
                    <details class="history-technical"><summary>Technical details</summary><div class="settings-row-v">Run <code>{run().id}</code>{run().sourceId ? <> · source <code>{run().sourceId}</code></> : null}{run().scope ? <> · workspace folder <code>{run().scope}</code></> : null}</div></details></div>
                  <button class="btn" disabled={!!busy() || run().changes.length === 0 || run().status === 'running' || !runRecoverable(run()) || sharedChanges(run()).length > 0} title={!runRecoverable(run()) ? 'History can’t restore every file in this run; see the list above the files.' : sharedChanges(run()).length > 0 ? 'Another conversation also changed some of these files; restore them one at a time.' : undefined} onClick={() => restoreRun(run())}>Restore entire run</button>
                </div>
                {/* Merged plan slice 4: the restore preview, before any restore. */}
                <Show when={nonEmpty(restorePreview(run().changes))}>
                  {blocked => (
                    <div class="settings-banner warn history-unrecoverable" role="note">
                      <strong>Not fully recoverable</strong>
                      <p>History can’t restore {blocked().length === 1 ? 'this file' : `these ${blocked().length} files`}, so it can’t restore the run as a whole. The run’s other files restore one at a time.</p>
                      <ul class="history-flag-list">
                        <For each={blocked()}>{item => <li><code>{item.path}</code>: {item.reason}</li>}</For>
                      </ul>
                    </div>
                  )}
                </Show>
                {/* Parallel chats in one workspace (2026-10-07). */}
                <Show when={(run().overlaps ?? []).length > 0}>
                  <div class="settings-banner history-overlaps" role="note">
                    <strong>{overlapLine(run(), id => { const peer = (runs() ?? []).find(r => r.id === id); return peer ? runTitle(peer, wid => workspaces().find(w => w.id === wid)?.title) : undefined; })}</strong>
                    <Show when={nonEmpty(sharedChanges(run()))} fallback={<p>No file was changed by both.</p>}>
                      {shared => (
                        <>
                          <p>{shared().length === 1 ? 'This file was' : `These ${shared().length} files were`} changed by both, and History can’t tell whose edit was whose. Hollow asks before restoring {shared().length === 1 ? 'it' : 'them'}, since that could undo the other conversation’s work; restore the run’s other files one at a time.</p>
                          <ul class="history-flag-list">
                            <For each={shared()}>{change => <li><code>{change.vaultPath}</code></li>}</For>
                          </ul>
                        </>
                      )}
                    </Show>
                    <div class="history-overlap-links">
                      <For each={(run().overlaps ?? []).flatMap(id => (runs() ?? []).filter(r => r.id === id))}>
                        {peer => <button class="panel-link" onClick={() => { setSelectedId(peer.id); setPreview(null); }}>{runTitle(peer, wid => workspaces().find(w => w.id === wid)?.title)} · {fmtWhen(peer.startedAt)}</button>}
                      </For>
                    </div>
                  </div>
                </Show>
                <Show when={run().changes.some(change => change.notionMirror)}>
                  <div class="settings-banner warn history-notion-warning" role="note">
                    <strong>Imported notes belong to your vault.</strong>
                    <p>Restoring an imported note changes the local file. Notion pages stay unchanged, and future imports preserve your restored content.</p>
                  </div>
                </Show>
                <Show when={protectedFlags(run()).length > 0}>
                  <div class="settings-banner warn history-run-flags" role="note">
                    <strong>Protected changes in this run</strong>
                    <p>Hollow did not make these changes itself. A changed automation is paused until you approve it in Automations → Permission changes; restore anything else from this run.</p>
                    <ul class="history-flag-list">
                      <For each={protectedFlags(run())}>{flag => <li>{runFlagLine(flag)}</li>}</For>
                    </ul>
                  </div>
                </Show>
                <Show when={sharedDocFlags(run()).length > 0}>
                  <div class="settings-banner warn history-run-flags" role="note">
                    <strong>A shared file was written over</strong>
                    <p>This run wrote a workspace file back from an older copy after another conversation or Hollow changed it. Each line below says what Hollow did about it.</p>
                    <ul class="history-flag-list">
                      <For each={sharedDocFlags(run())}>{flag => <li>{runFlagLine(flag)}</li>}</For>
                    </ul>
                  </div>
                </Show>
                <div class="history-change-list">
                  <For each={run().changes}>
                    {change => (
                      <div class="history-change-row">
                        <span class={`history-change-kind ${change.changeType}`}>{change.changeType}</span>
                        <button class="panel-link history-change-path" onClick={() => openFile(change.vaultPath)}>{change.vaultPath}</button>
                        <Show when={(change.alsoChangedBy ?? []).length > 0}><span class="settings-state-chip" data-state="warn" title="Another conversation in this workspace changed this file at the same time">shared</span></Show>
                        <Show when={change.unattributed}><span class="settings-state-chip" title="None of this run's own edits named this file">another chat</span></Show>
                        <Show when={change.unverified}><span class="settings-state-chip" data-state="warn" title="This run ended with Hollow's process. Changes unknown: this may include edits you made while Hollow was down.">changes unknown</span></Show>
                        <span class="history-change-size">{coverageLabel(sideCoverage(change, 'after')) || (change.afterSize == null ? 'deleted' : fmtBytes(change.afterSize))}</span>
                        <button class="btn ghost sm" disabled={!change.text || !!busy() || !!uncapturedReason(change)} onClick={() => loadPreview(run(), change)}>{busy() === `${run().id}:${change.vaultPath}:preview` ? 'Opening…' : 'Preview'}</button>
                        <button class="btn ghost sm" disabled={!!busy() || run().status === 'running' || !!uncapturedReason(change)} title={uncapturedReason(change) ?? undefined} onClick={() => restoreChange(run(), change)}>Restore</button>
                      </div>
                    )}
                  </For>
                </div>
              </>
            )}
          </Show>
        </div>
      </div>
      <Show when={preview()}>
        {item => (
          <div class="history-preview" role="region" aria-label={`History preview for ${item().path}`}>
            <div class="history-preview-head"><strong>{item().path}</strong><button class="btn ghost sm" onClick={() => setPreview(null)}>Close preview</button></div>
            <div class="history-preview-grid">
              <div><span class="history-preview-label">Before</span><pre>{item().before ?? '∅ file did not exist'}</pre></div>
              <div><span class="history-preview-label">After</span><pre>{item().after ?? '∅ file did not exist'}</pre></div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}
