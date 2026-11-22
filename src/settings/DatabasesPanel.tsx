import { For, Show, createSignal } from 'solid-js';
import { api } from '../api';
import type { HollowDatabases } from '../types';
import { checkFailed, checkWords, dbLine, dbSummary, staleLine } from '../hollowDatabases';
import { ErrorText } from '../components/ui';
import { errMsg } from '../utils';

/**
 * Settings, Advanced, Hollow's databases (repo features R7b): the SQLite
 * files in Hollow's app support folder (conversations, the search and
 * meaning-search indexes, the companion ledger), each with whose it is, its
 * size, WAL and version (`hollow_databases`). Measure reads sizes and each
 * header; Check integrity also runs SQLite's quick_check and counts each
 * table's rows on read-only connections (seconds on a large file). Both
 * change nothing. It has no delete button: other vaults' old indexes are
 * removed from Disk space → Remove old indexes…, which its last line names.
 * Loaded with Settings' System section only (lazy).
 */
export function DatabasesPanel() {
  const [report, setReport] = createSignal<HollowDatabases | null>(null);
  const [busy, setBusy] = createSignal<'' | 'measure' | 'check'>('');
  const [error, setError] = createSignal('');
  // Which button made the shown report (A4): only a check's summary counts
  // the files it couldn't open.
  const [checked, setChecked] = createSignal(false);
  async function load(check: boolean) {
    if (busy()) return;
    setBusy(check ? 'check' : 'measure');
    setError('');
    try {
      const next = await api.hollowDatabases(check);
      setChecked(check);
      setReport(next);
    }
    catch (e) { setError(errMsg(e)); }
    finally { setBusy(''); }
  }
  const status = () => busy() === 'check' ? 'Checking each file (seconds on a large one)…'
    : busy() ? "Measuring Hollow's databases…" : report() ? dbSummary(report()!, checked()) : '';
  return <>
    <div class="settings-actions">
      <button class="btn" aria-disabled={!!busy()} aria-busy={busy() === 'measure'} onClick={() => void load(false)}>Measure</button>
      <button class="btn" aria-disabled={!!busy()} aria-busy={busy() === 'check'} onClick={() => void load(true)}>Check integrity</button>
    </div>
    {/* Always mounted, so each result is announced. */}
    <p class="disk-note" role="status">{status()}</p>
    <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>
    <Show when={report()}>{r => <>
      <ul class="disk-folders" role="list" aria-label="Hollow's databases">
        <For each={r().files}>{f =>
          <li>
            <span class="disk-folder-path" title={f.path}>{f.path.split('/').pop()}</span>{' '}
            <span class="disk-folder-size">{dbLine(f)}</span>
            <Show when={checkWords(f)}>{w => <div class="disk-note" classList={{ 'settings-row-warn': checkFailed(f) || !!f.error }}>{w()}</div>}</Show>
          </li>}
        </For>
      </ul>
      <Show when={staleLine(r())}>{line => <p class="disk-note">{line()}</p>}</Show>
    </>}</Show>
  </>;
}
