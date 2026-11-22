import { For, Show, createEffect, createSignal, on, onMount } from 'solid-js';
import { api } from '../api';
import type { DiskReport } from '../types';
import { NOTHING_STALE, NOTHING_TO_PRUNE, cleanConfirmBody, cleanConfirmTitle, cleanedText, cleanupErrorText, diskSummary, folderSize, nothingToCleanText, parseThreshold, prunedText, pruneConfirmBody, pruneConfirmTitle, removedIndexesText, staleConfirmBody, staleConfirmTitle, withThreshold } from '../diskGuard';
import { ErrorText } from '../components/ui';
import { showConfirm, trackTask } from '../store';
import { errMsg } from '../utils';
import { registerDirty } from './settingsDirty';

// A cleanup previewing or running, and what the last one said. Module-level,
// not per panel: leaving Settings mid-delete and coming back mounts a new
// panel, which must see the delete still running (its buttons wait) rather
// than start a second one, and shows how it ended.
const [cleaning, setCleaning] = createSignal<'' | 'prune' | 'clean' | 'indexes'>('');
const [cleanNote, setCleanNote] = createSignal('');
const [cleanError, setCleanError] = createSignal('');
// Bumped when a delete has run: whichever panel is mounted then measures
// again. The panel that started it may be gone (Settings left and reopened
// mid-delete), so its own `check` would update nothing on screen (EH10).
const [deleted, setDeleted] = createSignal(0);

/**
 * Settings, Advanced, Disk space (overnight D1): free space against the
 * threshold Hollow warns at, and Hollow's largest folders. On demand, since
 * sizing runs `du` for up to 20 s. Measuring deletes nothing.
 *
 * Its two cleanup buttons (overnight H20) are the person's click for the
 * confirm-tier app actions agents reach with a card: Delete old install
 * builds… (`build.prune_install_targets`) and Remove finished worktrees…
 * (`build.clean_worktrees`). Each previews first (`install_targets`,
 * `worktree_hygiene`), shows every path that goes and stays in the confirm
 * dialog, with sizes from the last measure, then deletes exactly that list:
 * the preview's hash binds it, so a list that changed since refuses and
 * nothing goes. Then it measures again. The third, Remove old indexes…
 * (repo features R7b, `system.remove_stale_indexes`), does the same for
 * other vaults' search indexes (`stale_indexes`).
 */
export function DiskSpacePanel() {
  const [report, setReport] = createSignal<DiskReport | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [threshold, setThreshold] = createSignal('');
  // Typed into since the last check or save: a check doesn't overwrite it.
  const [edited, setEdited] = createSignal(false);
  const [saved, setSaved] = createSignal('');
  const [fieldError, setFieldError] = createSignal('');
  // The saved threshold before any measure, so Warn under shows at once (P1-22).
  const [savedGb, setSavedGb] = createSignal<number | null>(null);
  onMount(() => {
    api.diskThreshold().then(gb => { setSavedGb(gb); if (!edited() && !report()) setThreshold(String(gb)); }).catch(() => { /* shown after a check instead */ });
  });
  // A typed threshold that differs from the saved one is a draft (P1-21).
  registerDirty('disk-threshold', () => edited() && threshold().trim() !== String(report()?.thresholdGb ?? savedGb() ?? ''), 'Disk space warning');
  async function saveThreshold() {
    if (busy()) return;
    const parsed = parseThreshold(threshold());
    setSaved('');
    if ('error' in parsed) { setFieldError(parsed.error); return; }
    setFieldError('');
    try {
      const gb = await api.setDiskThreshold(parsed.gb);
      setSavedGb(gb);
      setEdited(false);
      setSaved(`Hollow now warns under ${gb} GB.`);
      // The last measure under the new threshold: no need to size again.
      const r = report();
      if (r) setReport(withThreshold(r, gb));
    } catch (e) { setFieldError(e instanceof Error ? e.message : String(e)); }
  }
  createEffect(on(deleted, () => void check(), { defer: true }));
  async function check() {
    if (busy() || cleaning()) return;
    setBusy(true);
    setError('');
    try {
      const r = await api.diskGuard();
      setReport(r);
      // The saved value (an agent may have changed it), unless being edited.
      if (!edited()) setThreshold(String(r.thresholdGb));
    }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  }
  /** Preview, confirm, delete exactly the previewed list, measure again.
   *  Cancel (or Esc, or the backdrop) deletes nothing. */
  async function cleanup(which: 'prune' | 'clean' | 'indexes') {
    if (busy() || cleaning()) return;
    setCleaning(which);
    setCleanNote({ prune: 'Reading the install build folders…', clean: 'Reading the worktrees (this can take some seconds)…', indexes: "Reading Hollow's indexes…" }[which]);
    setCleanError('');
    let done = '';
    let ran = false;
    try {
      if (which === 'prune') {
        const plan = await api.installTargets();
        if (!plan.remove.length) { done = NOTHING_TO_PRUNE; return; }
        setCleanNote('');
        const n = plan.remove.length;
        const ok = await showConfirm({
          title: pruneConfirmTitle(plan),
          body: pruneConfirmBody(plan, report()), confirmLabel: 'Delete', danger: true,
        });
        if (!ok) { done = 'Nothing was deleted.'; return; }
        setCleanNote('Deleting…');
        ran = true;
        done = prunedText(await trackTask('Delete old install builds', () => api.pruneInstallTargets(null, plan.hash), {
          detail: `${n} folder${n === 1 ? '' : 's'}`,
          // Some folders couldn't go: amber in the activity dock, not a green check.
          onSuccessStatus: o => o.failed.length ? 'warning' : 'success',
        }));
      } else if (which === 'indexes') {
        // Repo features R7b: other vaults' search indexes (rebuildable).
        const plan = await api.staleIndexes();
        if (!plan.files.length) { done = NOTHING_STALE; return; }
        setCleanNote('');
        const n = plan.files.length;
        const ok = await showConfirm({ title: staleConfirmTitle(plan), body: staleConfirmBody(plan), confirmLabel: 'Remove', danger: true });
        if (!ok) { done = 'Nothing was removed.'; return; }
        setCleanNote('Removing…');
        ran = true;
        done = removedIndexesText(await trackTask('Remove old indexes', () => api.removeStaleIndexes(plan.hash), {
          detail: `${n} index${n === 1 ? '' : 'es'}`,
          onSuccessStatus: o => o.failed.length ? 'warning' : 'success',
        }));
      } else {
        const r = await api.worktreeHygiene();
        if (!r.remove.length) { done = nothingToCleanText(r); return; }
        setCleanNote('');
        const n = r.remove.length;
        const ok = await showConfirm({
          title: cleanConfirmTitle(r),
          body: cleanConfirmBody(r, report()), confirmLabel: 'Remove', danger: true,
        });
        if (!ok) { done = 'Nothing was removed.'; return; }
        setCleanNote('Removing…');
        ran = true;
        done = cleanedText(await trackTask('Remove finished worktrees', () => api.cleanWorktrees(null, r.hash), {
          detail: `${n} path${n === 1 ? '' : 's'}`,
          onSuccessStatus: o => o.failed.length ? 'warning' : 'success',
        }));
      }
    } catch (e) {
      // The list changed since the dialog (hash mismatch): say so plainly.
      setCleanError(cleanupErrorText(errMsg(e), which === 'prune' ? 'deleted' : 'removed'));
    } finally {
      setCleanNote(done);
      setCleaning('');
      // Once a delete ran (even one that failed part-way), free space may
      // have changed: the mounted panel measures again.
      if (ran) setDeleted(n => n + 1);
    }
  }
  return <>
    <div class="settings-actions">
      <button class="btn" aria-disabled={busy() || !!cleaning()} aria-busy={busy()} onClick={() => void check()}>
        {busy() ? 'measuring…' : report() ? 'Check again' : 'Check disk space'}
      </button>
      {/* The threshold the daily bell uses (morning N6), read without measuring (P1-22). */}
      <Show when={report() || savedGb() !== null}>
        <label class="disk-threshold">Warn under
          <input class="settings-input" type="text" inputmode="numeric" value={threshold()} style={{ width: '5rem' }}
            aria-invalid={!!fieldError()} aria-describedby={fieldError() ? 'disk-threshold-error' : undefined}
            onInput={(e) => { setThreshold(e.currentTarget.value); setEdited(true); setFieldError(''); setSaved(''); }}
            onKeyDown={(e) => { if (e.key === 'Enter') void saveThreshold(); }} /> GB
        </label>
        <button class="btn ghost small" aria-label="Save the warning threshold" aria-disabled={busy()} onClick={() => void saveThreshold()}>Save</button>
      </Show>
    </div>
    <Show when={fieldError()}>{msg => <div id="disk-threshold-error"><ErrorText class="settings-row-warn" text={msg()}/></div>}</Show>
    <Show when={saved()}><p class="disk-note" role="status">{saved()}</p></Show>
    {/* Always mounted, so the first result is announced too. */}
    <p class="disk-note" classList={{ 'settings-row-warn': !!report()?.low }} role="status">{busy() ? 'Measuring Hollow\'s folders…' : report() ? diskSummary(report()!) : ''}</p>
    <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()}/>}</Show>
    <Show when={report()}>{r =>
      <ul class="disk-folders" role="list" aria-label="Hollow's largest folders">
        <For each={r().folders}>{f =>
          <li><span class="disk-folder-path" title={f.path}>{f.path}</span> <span class="disk-folder-size">{folderSize(f)}</span></li>}
        </For>
      </ul>}
    </Show>
    {/* Overnight H20: the two cleanups, each previewed in a confirm dialog. */}
    <p class="disk-note">Each cleanup shows exactly what goes, then asks first.</p>
    <div class="settings-actions">
      <button class="btn" aria-disabled={busy() || !!cleaning()} aria-busy={cleaning() === 'prune'} onClick={() => void cleanup('prune')}>Delete old install builds…</button>
      <button class="btn" aria-disabled={busy() || !!cleaning()} aria-busy={cleaning() === 'clean'} onClick={() => void cleanup('clean')}>Remove finished worktrees…</button>
      <button class="btn" aria-disabled={busy() || !!cleaning()} aria-busy={cleaning() === 'indexes'} onClick={() => void cleanup('indexes')}>Remove old indexes…</button>
    </div>
    {/* Always mounted, so each step is announced. */}
    <p class="disk-note" role="status">{cleanNote()}</p>
    <Show when={cleanError()}>{msg => <ErrorText class="settings-row-warn" text={msg()}/>}</Show>
  </>;
}
