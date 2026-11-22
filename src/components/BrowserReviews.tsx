/**
 * Page reviews in the side panel browser (next-level P15): the annotation
 * sets saved to this workspace (Rust `workspace_reviews.rs`), each capture
 * labelled as a capture (time and viewport), the requested changes with the
 * capture that shows their result, and "Compare with a newer capture": a
 * before/after where the user marks each requested change Done, Not yet or
 * Unclear. Lives inside `.browser-page`, so the native page hides while it is
 * open (BrowserPane's overlay rule).
 */
import { For, Index, Show, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import type { AnnotationSet, ReviewCapture, ReviewVerdict } from '../types';
import { markAnchor, type Mark, type SpatialMark } from '../annotate';
import {
  VERDICT_LABEL, captureLabel, captureTime, progressLabel, requestedChanges, reviewProgress,
} from '../browserReview';
import { Button, IconButton, ListRow, Section, Segmented, Skeleton, StatePanel, StatusPill } from './ui';
import { flashFooter } from '../store';
import { errMsg } from '../utils';

/** A capture the pane just took for a comparison. */
export interface Shot { image: string; view: { width: number; height: number }; imageSize: { width: number; height: number }; url: string; title: string; capturedAt: string }

const VERDICT_TONE: Record<ReviewVerdict, string> = { done: 'ok', notDone: 'warn', unclear: 'neutral' };

const host = (url: string | null | undefined) => { try { return url ? new URL(url).host || url : ''; } catch { return url ?? ''; } };

/** A capture with its marks drawn over it (numbers match the list). */
function MarkedCapture(props: { image: string | null; size: { width: number; height: number }; marks: Mark[]; label: string; alt: string }) {
  const w = () => Math.max(1, props.size.width), h = () => Math.max(1, props.size.height);
  const at = (m: SpatialMark) => { const p = m.kind === 'box' ? { x: m.x + m.w, y: m.y } : markAnchor(m); return { left: `${(p.x / w()) * 100}%`, top: `${(p.y / h()) * 100}%` }; };
  return (
    <figure class="review-capture">
      <div class="review-capture-frame" style={{ 'aspect-ratio': `${w()} / ${h()}` }}>
        <Show when={props.image} fallback={<div class="review-capture-missing">The capture image is missing.</div>}>
          <img src={props.image!} alt={props.alt} draggable={false} />
        </Show>
        <svg viewBox={`0 0 ${w()} ${h()}`} preserveAspectRatio="none" aria-hidden="true">
          <For each={props.marks}>{m =>
            m.kind === 'box' ? <rect class="annotate-ink" x={m.x} y={m.y} width={m.w} height={m.h} />
              : m.kind === 'draw' ? <path class="annotate-ink" d={m.points.map((p, i) => `${i ? 'L' : 'M'}${p[0]} ${p[1]}`).join(' ')} /> : null
          }</For>
        </svg>
        <Index each={props.marks}>{(m, i) => <Show when={m().kind !== 'page' ? m() as SpatialMark : null}>{spatial => <span class="review-mark-num" style={at(spatial())} aria-hidden="true">{i + 1}</span>}</Show>}</Index>
      </div>
      <figcaption class="review-capture-label">{props.label}</figcaption>
    </figure>
  );
}

export function BrowserReviews(props: {
  workspaceId: string;
  /** Open this review first (just saved). */
  initialPath?: string | null;
  /** A ready tab whose page can be captured now. */
  canCapture: boolean;
  capture: () => Promise<Shot | null>;
  onOpenUrl: (url: string) => void;
  onClose: () => void;
}) {
  const [list, { refetch }] = createResource(() => props.workspaceId, id => api.listAnnotationSets(id));
  const [set, setSet] = createSignal<AnnotationSet | null>(null);
  const [error, setError] = createSignal('');
  const [conflict, setConflict] = createSignal(false);
  const [shot, setShot] = createSignal<Shot | null>(null);
  const [verdicts, setVerdicts] = createSignal<Record<number, ReviewVerdict>>({});
  const [notes, setNotes] = createSignal<Record<number, string>>({});
  const [busy, setBusy] = createSignal(false);
  const [viewing, setViewing] = createSignal<string | null>(null);

  async function open(path: string) {
    setError(''); setConflict(false); setShot(null); setViewing(null);
    try { setSet(await api.readAnnotationSet(props.workspaceId, path)); }
    catch (e) { setError(errMsg(e)); }
  }
  if (props.initialPath) void open(props.initialPath);
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || (e.target as HTMLElement | null)?.closest?.('input, textarea')) return;
      e.preventDefault();
      if (shot()) setShot(null); else props.onClose();
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  const first = () => set()?.captures[0] ?? null;
  const shown = (): ReviewCapture | null => set()?.captures.find(c => c.id === viewing()) ?? set()?.captures.at(-1) ?? null;
  const capOf = (id: string | null | undefined) => set()?.captures.find(c => c.id === id) ?? null;
  const sizeOf = (c: ReviewCapture) => (c.imageSize.width ? c.imageSize : c.viewport);

  async function startCompare() {
    if (busy()) return;
    setBusy(true);
    try {
      const s = await props.capture();
      if (!s) return;
      const prior: Record<number, ReviewVerdict> = {};
      for (const c of requestedChanges(set()!)) prior[c.index] = c.link?.status ?? 'unclear';
      setVerdicts(prior); setNotes({}); setShot(s);
    } finally { setBusy(false); }
  }

  async function saveCompare() {
    const s = shot(), current = set();
    if (!s || !current || busy()) return;
    setBusy(true);
    try {
      const links = requestedChanges(current).map(c => ({ capture: current.captures[0].id, mark: c.index, status: verdicts()[c.index] ?? 'unclear', note: notes()[c.index] ?? '' }));
      const next = await api.addAnnotationCapture(props.workspaceId, current.path, current.sha256, {
        url: s.url, title: s.title, capturedAt: s.capturedAt, viewport: s.view, imageSize: s.imageSize, marks: [], pngBase64: s.image.replace(/^data:[^,]*,/, ''),
      }, links);
      setSet(next); setShot(null); setViewing(null);
      flashFooter(`Comparison saved: ${progressLabel(reviewProgress(next))}`, 4000);
      void refetch();
    } catch (e) {
      const msg = errMsg(e);
      if (msg.startsWith('REVIEW_CHANGED')) setConflict(true); else setError(msg);
    } finally { setBusy(false); }
  }

  return (
    <div class="browser-reviews" role="region" aria-label="Page reviews">
      <div class="annotate-bar">
        <Show when={set()} fallback={<strong class="browser-reviews-title">Page reviews</strong>}>
          <Button size="sm" variant="ghost" icon="arrowLeft" onClick={() => { setSet(null); setShot(null); void refetch(); }}>All reviews</Button>
          <strong class="browser-reviews-title">{set()!.title}</strong>
        </Show>
        <span class="annotate-spacer" />
        <IconButton icon="x" label="Close reviews (Esc)" onClick={props.onClose} />
      </div>
      <div class="browser-reviews-body">
        <Show when={error()}><StatePanel compact kind="failed" title="That didn't work" actions={<Button size="sm" onClick={() => { setError(''); void refetch(); }}>Try again</Button>} error={error()} /></Show>
        <Show when={conflict()}>
          <StatePanel compact kind="conflicted" title="This review changed elsewhere" actions={<Button size="sm" onClick={() => void open(set()!.path)}>Reopen it</Button>}>
            Another window or a sync wrote it after you opened it. Nothing was overwritten; reopen it and compare again.
          </StatePanel>
        </Show>

        {/* The list of saved reviews. */}
        <Show when={!set()}>
          <Show when={!list.loading} fallback={<Skeleton label="Loading reviews" rows={3} />}>
            <Show when={!list.error} fallback={<StatePanel compact kind="failed" title="Reviews could not be listed" actions={<Button size="sm" onClick={() => void refetch()}>Try again</Button>} error={list.error} />}>
              <Show when={list()?.length} fallback={
                <StatePanel compact kind="empty" title="No saved reviews yet">Annotate a page, then Save to workspace. The capture and its marks become a review in this workspace's Results, ready to compare with a later capture.</StatePanel>}>
                <div class="browser-reviews-list">
                  <For each={list()}>{r =>
                    <ListRow icon="squarePen" title={r.title} onClick={() => void open(r.path)} label={`Open review ${r.title}`}
                      meta={r.error ? r.error : `${host(r.url)} · ${progressLabel(r)} · ${r.captures} capture${r.captures === 1 ? '' : 's'}`}
                      trailing={<Show when={!r.error && r.requested}><StatusPill quiet label={r.done === r.requested ? 'All done' : `${r.requested - r.done} open`} tone={r.done === r.requested ? 'ok' : 'warn'} /></Show>} />
                  }</For>
                </div>
              </Show>
            </Show>
          </Show>
        </Show>

        {/* One review, and the before/after compare. */}
        <Show when={set() && !shot()}>
          <div class="browser-review-detail">
            <div class="browser-review-actions">
              <span class="browser-review-progress">{progressLabel(reviewProgress(set()!))}</span>
              <span class="annotate-spacer" />
              <Button size="sm" variant="ghost" icon="globe" onClick={() => props.onOpenUrl(first()!.url)}>Open page</Button>
              <Button size="sm" variant="primary" icon="compare" disabled={!props.canCapture || busy()} onClick={() => void startCompare()}
                title={props.canCapture ? 'Capture the page as it is now and mark what changed' : 'Open the page in a tab first'}>
                {busy() ? 'Capturing…' : 'Compare with a newer capture'}
              </Button>
            </div>
            <Show when={set()!.captures.length > 1}>
              <div class="browser-review-versions" role="group" aria-label="Captures">
                <For each={set()!.captures}>{(c, i) =>
                  <button type="button" aria-pressed={shown()?.id === c.id} onClick={() => setViewing(c.id)}>
                    {i() === 0 ? 'Before' : `After ${i()}`} · {captureTime(c.capturedAt)}
                  </button>
                }</For>
              </div>
            </Show>
            <Show when={shown()}>{c =>
              <MarkedCapture image={c().imageDataUrl} size={sizeOf(c())} marks={(c().id === first()?.id ? c().marks : []) as Mark[]}
                label={`${captureLabel(c().capturedAt, c().viewport)} · ${host(c().url)}`} alt={`Capture of ${c().title || c().url}`} />
            }</Show>
            <Section title="Requested changes" count={requestedChanges(set()!).length}>
              <Show when={requestedChanges(set()!).length} fallback={<p class="browser-review-empty">This capture has no marks.</p>}>
                <ol class="browser-review-changes">
                  <For each={requestedChanges(set()!)}>{c => {
                    const result = () => capOf(c.link?.result);
                    return <li>
                      <span class="review-mark-num static" aria-hidden="true">{c.index + 1}</span>
                      <span class="browser-review-change-main">
                        <span>{c.mark.note.trim() || '(no comment)'}</span>
                        <span class="browser-review-change-meta">
                          <Show when={c.link} fallback="Not compared yet">
                            <StatusPill quiet label={VERDICT_LABEL[c.link!.status]} tone={VERDICT_TONE[c.link!.status]} />
                            <Show when={result()}>
                              {' · '}<button type="button" class="btn-link" onClick={() => setViewing(result()!.id)}>in {captureLabel(result()!.capturedAt, result()!.viewport)}</button>
                            </Show>
                            <Show when={c.link!.note}> · {c.link!.note}</Show>
                          </Show>
                        </span>
                      </span>
                    </li>;
                  }}</For>
                </ol>
              </Show>
            </Section>
          </div>
        </Show>

        <Show when={set() && shot()}>
          <div class="browser-review-compare" role="group" aria-label="Before and after">
            <div class="browser-review-pair">
              <MarkedCapture image={first()!.imageDataUrl} size={sizeOf(first()!)} marks={first()!.marks as Mark[]} label={`Before · ${captureLabel(first()!.capturedAt, first()!.viewport)}`} alt="Before" />
              <MarkedCapture image={shot()!.image} size={shot()!.imageSize} marks={[]} label={`After · ${captureLabel(shot()!.capturedAt, shot()!.view)}`} alt="After" />
            </div>
            <ol class="browser-review-changes">
              <For each={requestedChanges(set()!)}>{c =>
                <li>
                  <span class="review-mark-num static" aria-hidden="true">{c.index + 1}</span>
                  <span class="browser-review-change-main">
                    <span>{c.mark.note.trim() || '(no comment)'}</span>
                    <span class="browser-review-verdict">
                      <Segmented label={`Change ${c.index + 1}`} value={verdicts()[c.index] ?? 'unclear'} onChange={v => setVerdicts(m => ({ ...m, [c.index]: v }))}
                        items={[{ value: 'done', label: 'Done' }, { value: 'notDone', label: 'Not yet' }, { value: 'unclear', label: 'Unclear' }]} />
                      <input class="browser-review-note" type="text" aria-label={`Note for change ${c.index + 1}`} placeholder="Note (optional)" value={notes()[c.index] ?? ''}
                        onInput={e => setNotes(m => ({ ...m, [c.index]: e.currentTarget.value }))} />
                    </span>
                  </span>
                </li>
              }</For>
            </ol>
            <div class="browser-review-actions">
              <span class="annotate-spacer" />
              <Button size="sm" variant="ghost" onClick={() => setShot(null)}>Cancel</Button>
              <Button size="sm" variant="primary" disabled={busy()} onClick={() => void saveCompare()}>{busy() ? 'Saving…' : 'Save comparison'}</Button>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
}

