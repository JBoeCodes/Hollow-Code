import { Show, Suspense, createSignal, lazy } from 'solid-js';
import { api } from '../api';
import { workspaceId, refreshWorkspace } from '../store/workspaces';
import { openVaultNote, vaultRoot } from '../store';
import { PlainErrorText } from './ui';
import { vaultRelativePath, type SourceState, opensAsDocument } from '../sourceAuthority';
import type { WorkspaceDetail } from '../workspaceTypes';
import '../styles/sources-cards.css';
const SourceRefresh = lazy(() => import('./SourceRefresh').then(m => ({ default: m.SourceRefresh })));

/** What a source is, summary first and the exact provenance after (plan L3 X3). */
export function SourceStateText(props: { state: SourceState; class?: string }) {
  return <p class={`source-state ${props.class ?? ''}`} data-kind={props.state.kind} data-tone={props.state.tone}><strong>{props.state.summary}</strong><span>{props.state.detail}</span></p>;
}

/** Snapshots replaced this session, by `workspaceId|path`: the card says so
 *  after the refreshed listing redraws it. */
const [replaced, setReplaced] = createSignal<Record<string, true>>({});

/**
 * A snapshot's three actions (plan L3 X4, 2026-10-14; review U library):
 * View snapshot (the copy, read only), Open original (in the Library reader
 * when it lives in the vault, else in its own app; disabled with the reason
 * when it can't be found) and Check for changes (the reviewed refresh,
 * `SourceRefresh`, loaded on first use).
 */
export function SourceActions(props: { workspaceId: string; path: string; name: string; original: string | null; freshness: string; refreshable: boolean; onView?: () => void; onReplaced?: (detail: WorkspaceDetail) => void }) {
  const [checking, setChecking] = createSignal(false);
  const [error, setError] = createSignal('');
  const [revealed, setRevealed] = createSignal(false);
  const key = () => `${props.workspaceId}|${props.path}`;
  const missing = () => !props.original ? 'No original recorded' : props.freshness === 'unavailable' ? 'The original can’t be found' : '';
  async function openOriginal() {
    const original = props.original; if (!original) return;
    setError(''); setRevealed(false);
    const rel = vaultRelativePath(original, await vaultRoot());
    if (rel) { openVaultNote(rel); return; }
    // The sidecar naming the original is a file an agent can edit, so an
    // app, script or extensionless file is shown in Finder, never launched
    // (L3 review, 2026-10-14). The name can lie (a link called report.pdf to
    // an app), so the backend decides again on the link's target and reveals
    // a folder, app, executable or script it would otherwise open (H4).
    // Only a reveal the backend chose is explained: one asked for here (an
    // extensionless file, a folder) isn't something that "would run".
    const reveal = !opensAsDocument(original);
    try { const how = await api.chatPreviewOpen(original, reveal); setRevealed(!reveal && how === 'revealed'); } catch (e) { setError(String(e)); }
  }
  async function done(detail: WorkspaceDetail) {
    setChecking(false); setReplaced(r => ({ ...r, [key()]: true }));
    if (workspaceId() === props.workspaceId) await refreshWorkspace();
    props.onReplaced?.(detail);
  }
  return <div class="source-actions">
    <div class="source-actions-row" role="group" aria-label={`Actions for ${props.name}`}>
      <Show when={props.onView}><button type="button" class="btn small" onClick={() => props.onView!()}>View snapshot</button></Show>
      <button type="button" class="btn small" disabled={!!missing()} title={missing() || 'Open the file this snapshot was copied from'} onClick={() => void openOriginal()}>Open original</button>
      <button type="button" class="btn small" aria-expanded={checking()} disabled={!props.refreshable} title={props.refreshable ? 'Compare the snapshot with its original by content' : 'Nothing records where this snapshot came from'} onClick={() => { setReplaced(r => { const next = { ...r }; delete next[key()]; return next; }); setChecking(v => !v); }}>Check for changes</button>
      <Show when={missing()}>{why => <small class="source-action-why">{why()}</small>}</Show>
    </div>
    <Show when={error()}><p class="work-muted" role="alert"><PlainErrorText text={error()}/></p></Show>
    <Show when={revealed()}><p class="work-muted" role="status">Shown in Finder: opening this original would run it, so Hollow only shows where it is.</p></Show>
    <Show when={replaced()[key()] && !checking()}><p class="source-replaced" role="status">Snapshot replaced. SOURCES.md records the refresh; the earlier snapshot is kept in sources/.previous/.</p></Show>
    <Show when={checking()}><Suspense fallback={<p class="work-muted" role="status">Comparing the snapshot with its original…</p>}><SourceRefresh workspaceId={props.workspaceId} path={props.path} name={props.name} onReplaced={d => void done(d)} onClose={() => setChecking(false)}/></Suspense></Show>
  </div>;
}
