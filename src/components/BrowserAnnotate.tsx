import { Index, Show, createEffect, createSignal, createUniqueId, on, onCleanup, onMount } from 'solid-js';
import { Icon } from '../icons';
import { api } from '../api';
import { appendToChatDraft, attachChatImage, attachSharedPage, chatAttachments, chatConversationVersion, flashFooter, showConfirm } from '../store';
import { ATTACH_MAX_COUNT } from '../turnFiles';
import { browserWorkspaceScope } from '../store/browser';
import type { SharedPage } from '../sharedPage';
import { annotationText, markAnchor, normalizeBox, toViewport, type Mark, type PageInspection, type SpatialMark } from '../annotate';
import { captureLabel, captureSentence, dataUrlBody, removeDraft, saveDraft, type AnnotationDraft } from '../browserReview';
import type { AnnotationSet } from '../types';
import { errMsg } from '../utils';
import { modalOpen } from '../modalGuard';
import { Segmented } from './ui';

type Tool = 'pin' | 'box' | 'draw';
/** The ink on the snapshot and in the sent image. A canvas colour, so not a
 *  CSS token: the image must look the same in every theme. */
const INK = '#ff3b30';
/** The sent image is scaled down to at most this width. */
const MAX_SENT_WIDTH = 1600;

export interface AnnotateTarget {
  tabId: string;
  /** `data:image/png;base64,…` from browser_snapshot. */
  image: string;
  /** The page's viewport (CSS px) when it was captured. */
  view: { width: number; height: number };
  url: string;
  title: string;
  /** When the snapshot was taken (next-level P15): it is a capture, labelled
   *  as one, never the live page. */
  capturedAt: string;
  /** The workspace (or `chat`) the draft belongs to. */
  scope: string;
  /** The snapshot's own size, when the image shown is a scaled-down copy
   *  (a restored draft); marks stay in these pixels. */
  size?: { width: number; height: number } | null;
  /** A restored draft's marks. */
  marks?: Mark[];
}

/** A data URL re-encoded through a canvas: as PNG (for saving), or scaled to
 *  `maxWidth` as JPEG (for a draft that has to fit in local storage). */
async function reencode(url: string, opts: { type: 'image/png' | 'image/jpeg'; maxWidth?: number }): Promise<string> {
  const img = new Image();
  await new Promise<void>((resolve, reject) => { img.onload = () => resolve(); img.onerror = () => reject(new Error('The capture could not be read.')); img.src = url; });
  const scale = Math.min(1, (opts.maxWidth ?? img.naturalWidth) / Math.max(1, img.naturalWidth));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  canvas.getContext('2d')!.drawImage(img, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL(opts.type, 0.82);
}

/**
 * Annotate (the side panel browser): the page frozen as a snapshot, with
 * numbered comments, boxes and freehand marks drawn over it. "Add to chat"
 * attaches the marked-up image to the composer and drafts one line per mark,
 * naming the page element under it (browser_inspect), so the agent can find
 * it in the code. Nothing is sent until the user sends the message.
 */
export function BrowserAnnotate(props: { target: AnnotateTarget; onClose: () => void; workspaceId?: string | null; onSaved?: (set: AnnotationSet) => void }) {
  const [tool, setTool] = createSignal<Tool>('pin');
  const [marks, setMarks] = createSignal<Mark[]>(props.target.marks ?? []);
  const [saving, setSaving] = createSignal(false);
  const [draft, setDraft] = createSignal<Mark | null>(null);
  const [editing, setEditing] = createSignal<number | null>(null);
  const [size, setSize] = createSignal<{ width: number; height: number } | null>(null);
  const [sending, setSending] = createSignal(false);
  /** Include page text (repo ideas 2026-10-27 #3): off until ticked, read
   *  only on Add to chat. */
  const [includeText, setIncludeText] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  /** Marks taken away by Undo or Remove, newest last, with where they stood. */
  const [removed, setRemoved] = createSignal<{ mark: Mark; at: number }[]>([]);
  const [cursor, setCursor] = createSignal<[number, number] | null>(null);
  const [stageFocused, setStageFocused] = createSignal(false);
  const [corner, setCorner] = createSignal<[number, number] | null>(null);
  const instructionsId = createUniqueId();
  const entries: HTMLButtonElement[] = [];
  let viewport!: HTMLDivElement;
  let stage: HTMLDivElement | undefined;
  let svg!: SVGSVGElement;
  let start: [number, number] | null = null;

  // The capture may be taller than its viewport. Follow only keyboard
  // movement, inside this scroller, without moving focus or the document.
  createEffect(() => {
    const p = cursor(), s = size();
    if (!stageFocused() || tool() === 'draw' || !p || !s) return;
    queueMicrotask(() => {
      if (!stageFocused() || document.activeElement !== stage || !stage) return;
      const image = stage.getBoundingClientRect(), view = viewport.getBoundingClientRect();
      const x = image.left + p[0] / s.width * image.width;
      const y = image.top + p[1] / s.height * image.height;
      const inset = 12;
      const dx = x < view.left + inset ? x - view.left - inset : x > view.right - inset ? x - view.right + inset : 0;
      const dy = y < view.top + inset ? y - view.top - inset : y > view.bottom - inset ? y - view.bottom + inset : 0;
      viewport.scrollBy({ left: dx, top: dy, behavior: 'instant' });
    });
  });

  onMount(() => {
    const img = new Image();
    img.onload = () => setSize(props.target.size ?? { width: img.naturalWidth, height: img.naturalHeight });
    img.src = props.target.image;
    // Only keys meant for the snapshot: pressed on it, or with nothing
    // focused, never one already handled or behind a dialog (Discard's own
    // question). ⌘Z in the composer stays the composer's (P1-47).
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as Element | null;
      if (e.defaultPrevented || modalOpen() || editing() !== null) return;
      if (t !== document.body && t !== document.documentElement && !t?.closest?.('.browser-annotate')) return;
      if (e.key === 'Escape') { e.preventDefault(); props.onClose(); }
      else if (e.metaKey && !e.altKey && !e.ctrlKey && e.key.toLowerCase() === 'z') { e.preventDefault(); if (e.shiftKey) redo(); else undo(); }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  // Drafts (next-level P15): every change to the marks is kept, so switching
  // tab, view or workspace, or a reload, never loses them. Sent, saved or
  // discarded removes the draft.
  const draftId = { scope: props.target.scope, tabId: props.target.tabId, capturedAt: props.target.capturedAt };
  let finished = false;
  let shrunk: string | null = null;
  let draftTimer: number | undefined;
  const persist = async () => {
    if (finished || !size()) return;
    if (!marks().length) { removeDraft(localStorage, draftId); return; }
    const draft: AnnotationDraft = {
      ...draftId, url: props.target.url, title: props.target.title, view: props.target.view, imageSize: size(),
      image: shrunk ?? props.target.image, marks: marks(), savedAt: new Date().toISOString(),
    };
    if (saveDraft(localStorage, draft) || shrunk) return;
    // Too big for local storage: keep a smaller copy of the capture.
    try { shrunk = await reencode(props.target.image, { type: 'image/jpeg', maxWidth: 1200 }); } catch { return; }
    if (!finished) saveDraft(localStorage, { ...draft, image: shrunk });
  };
  createEffect(on(marks, () => { window.clearTimeout(draftTimer); draftTimer = window.setTimeout(() => void persist(), 250); }, { defer: true }));
  onCleanup(() => { window.clearTimeout(draftTimer); if (!finished) void persist(); });
  const done = () => { finished = true; window.clearTimeout(draftTimer); removeDraft(localStorage, draftId); };
  /** Discard deletes the marks for good, so it asks first (P1-47). */
  const discard = async () => {
    const n = marks().length;
    const ok = await showConfirm({
      title: `Discard ${n === 1 ? '1 mark' : `${n} marks`}?`,
      body: 'The marks and their comments are deleted, and no draft is kept. Close keeps them instead.',
      confirmLabel: 'Discard', danger: true,
    });
    if (ok) { done(); props.onClose(); }
  };
  const label = () => captureLabel(props.target.capturedAt, props.target.view);

  /** A pointer event in snapshot pixels. */
  const at = (e: PointerEvent): [number, number] => {
    const r = svg.getBoundingClientRect(), s = size()!;
    const x = Math.min(Math.max(e.clientX - r.left, 0), r.width), y = Math.min(Math.max(e.clientY - r.top, 0), r.height);
    return [(x / r.width) * s.width, (y / r.height) * s.height];
  };
  /** Minimum drag, in snapshot pixels, for a box or a stroke to count. */
  const minDrag = () => (size()?.width ?? 1000) / 120;

  const add = (m: Mark, edit: boolean) => {
    setRemoved([]);
    setMarks(list => [...list, m]);
    if (edit) setEditing(marks().length - 1);
  };
  const focusEntry = (i: number) => queueMicrotask(() => {
    if (marks().length) entries[Math.min(i, marks().length - 1)]?.focus();
    else stage?.focus();
  });
  const finishEditing = (i: number) => { setEditing(null); focusEntry(i); };
  /** Take mark `i` away, kept so Redo (⇧⌘Z) can put it back where it was. */
  const take = (i: number) => {
    const list = marks();
    if (i < 0 || i >= list.length) return;
    setEditing(null);
    setRemoved(r => [...r, { mark: list[i], at: i }]);
    setMarks(list.filter((_, j) => j !== i));
  };
  const undo = () => take(marks().length - 1);
  const remove = (i: number) => { take(i); focusEntry(i); };
  const redo = () => {
    const last = removed().at(-1);
    if (!last) return;
    setEditing(null);
    setRemoved(r => r.slice(0, -1));
    setMarks(list => [...list.slice(0, last.at), last.mark, ...list.slice(last.at)]);
  };
  const setNote = (i: number, note: string) => setMarks(list => list.map((m, j) => (j === i ? { ...m, note } : m)));

  const cancelPending = () => { setCorner(null); setDraft(null); start = null; };
  const enterStage = () => {
    const s = size();
    if (s && !cursor()) setCursor([s.width / 2, s.height / 2]);
    setStageFocused(true);
  };
  const stageKey = (e: KeyboardEvent) => {
    if (e.target !== stage || e.defaultPrevented || modalOpen() || e.isComposing || e.metaKey || e.ctrlKey || e.altKey || !size()) return;
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter'].includes(e.key)) enterStage();
    if (e.key === 'Escape' && draft()) { e.preventDefault(); e.stopPropagation(); cancelPending(); return; }
    const s = size()!, p = cursor() ?? [s.width / 2, s.height / 2];
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) {
      e.preventDefault(); e.stopPropagation();
      const step = e.shiftKey ? 40 : 8;
      const next: [number, number] = [
        Math.max(0, Math.min(s.width, p[0] + (e.key === 'ArrowRight' ? step : e.key === 'ArrowLeft' ? -step : 0))),
        Math.max(0, Math.min(s.height, p[1] + (e.key === 'ArrowDown' ? step : e.key === 'ArrowUp' ? -step : 0))),
      ];
      setCursor(next);
      const c = corner();
      if (c) setDraft({ kind: 'box', ...normalizeBox(c[0], c[1], next[0], next[1]), note: '' });
    } else if (e.key === 'Enter') {
      e.preventDefault(); e.stopPropagation();
      if (e.repeat) return;
      if (tool() === 'pin') add({ kind: 'pin', x: p[0], y: p[1], note: '' }, true);
      else if (tool() === 'box') {
        const c = corner();
        if (!c) { setCorner(p); setDraft({ kind: 'box', x: p[0], y: p[1], w: 0, h: 0, note: '' }); }
        else {
          const box = normalizeBox(c[0], c[1], p[0], p[1]);
          if (box.w > minDrag() && box.h > minDrag()) { cancelPending(); add({ kind: 'box', ...box, note: '' }, true); }
        }
      }
    }
  };
  const entryKey = (e: KeyboardEvent, i: number) => {
    if (e.defaultPrevented || modalOpen() || e.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Delete' || e.key === 'Backspace') { e.preventDefault(); e.stopPropagation(); remove(i); }
  };

  const down = (e: PointerEvent) => {
    if (!size() || e.button !== 0) return;
    setStageFocused(false);
    cancelPending();
    setEditing(null);
    const p = at(e);
    if (tool() === 'pin') { add({ kind: 'pin', x: p[0], y: p[1], note: '' }, true); return; }
    start = p;
    svg.setPointerCapture(e.pointerId);
    setDraft(tool() === 'box' ? { kind: 'box', x: p[0], y: p[1], w: 0, h: 0, note: '' } : { kind: 'draw', points: [p], note: '' });
  };
  const move = (e: PointerEvent) => {
    const d = draft();
    if (!d || !start) return;
    const p = at(e);
    if (d.kind === 'box') setDraft({ ...d, ...normalizeBox(start[0], start[1], p[0], p[1]) });
    else if (d.kind === 'draw') setDraft({ ...d, points: [...d.points, p] });
  };
  const up = () => {
    const d = draft();
    setDraft(null);
    start = null;
    if (!d) return;
    if (d.kind === 'box' && d.w > minDrag() && d.h > minDrag()) add(d, true);
    if (d.kind === 'draw' && d.points.length > 2) add(d, false);
  };

  /** Where a mark's number and note sit, in % of the snapshot. */
  const labelAt = (m: SpatialMark) => {
    const s = size()!;
    const p = m.kind === 'box' ? { x: m.x + m.w, y: m.y } : markAnchor(m);
    return { left: `clamp(11px, ${(p.x / s.width) * 100}%, calc(100% - 11px))`, top: `clamp(11px, ${(p.y / s.height) * 100}%, calc(100% - 11px))` };
  };
  const path = (m: Extract<Mark, { kind: 'draw' }>) => m.points.map((p, i) => `${i ? 'L' : 'M'}${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(' ');

  /** The snapshot with the marks burned in, as a PNG File. */
  async function composite(): Promise<File> {
    const s = size()!;
    const img = new Image();
    await new Promise<void>((resolve, reject) => { img.onload = () => resolve(); img.onerror = () => reject(new Error('The capture could not be read.')); img.src = props.target.image; });
    const scale = Math.min(1, MAX_SENT_WIDTH / s.width);
    const canvas = document.createElement('canvas');
    canvas.width = Math.round(s.width * scale);
    canvas.height = Math.round(s.height * scale);
    const g = canvas.getContext('2d')!;
    g.drawImage(img, 0, 0, canvas.width, canvas.height);
    g.scale(scale, scale);
    const unit = s.width / 400;
    g.strokeStyle = INK; g.fillStyle = INK; g.lineWidth = unit; g.lineJoin = 'round'; g.lineCap = 'round';
    marks().forEach((m, i) => {
      if (m.kind === 'page') return;
      if (m.kind === 'box') g.strokeRect(m.x, m.y, m.w, m.h);
      if (m.kind === 'draw') { g.beginPath(); m.points.forEach((p, j) => (j ? g.lineTo(p[0], p[1]) : g.moveTo(p[0], p[1]))); g.stroke(); }
      const p = m.kind === 'box' ? { x: m.x + m.w, y: m.y } : markAnchor(m);
      const r = unit * 5;
      g.beginPath(); g.arc(p.x, p.y, r, 0, Math.PI * 2); g.fillStyle = INK; g.fill();
      g.fillStyle = '#ffffff'; g.font = `600 ${Math.round(r * 1.2)}px -apple-system, system-ui, sans-serif`;
      g.textAlign = 'center'; g.textBaseline = 'middle'; g.fillText(String(i + 1), p.x, p.y + r * 0.05);
    });
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/png'));
    if (!blob) throw new Error('The marked-up image could not be made.');
    let host = 'page';
    try { host = new URL(props.target.url).host.replace(/[^\w.-]+/g, '-') || host; } catch { /* keep "page" */ }
    return new File([blob], `annotated-${host}.png`, { type: 'image/png' });
  }

  async function send() {
    if (!size() || sending()) return;
    setSending(true);
    setEditing(null);
    // Whose composer it goes to, fixed now: a switch of conversation or
    // workspace while the page is read must never hand it to another one
    // (review of #1052).
    const version = chatConversationVersion();
    const scope = browserWorkspaceScope();
    const moved = () => disposed || version !== chatConversationVersion() || scope !== browserWorkspaceScope();
    try {
      // The page's text first: if the page doesn't answer, nothing is added
      // and Annotate stays open with its marks.
      let page: SharedPage | null = null;
      if (includeText()) page = { ...(await api.browserPageText(props.target.tabId)), sharedAt: new Date().toISOString() };
      const file = await composite();
      const spatial = marks().flatMap((m, index) => m.kind === 'page' ? [] : [{ index, point: toViewport(markAnchor(m), size()!, props.target.view) }]);
      let inspection: PageInspection | null = null;
      if (spatial.length) {
        const result = await api.browserInspect(props.target.tabId, spatial.map(m => m.point)).catch(() => null);
        if (result) {
          const elements: PageInspection['elements'] = marks().map(() => null);
          spatial.forEach((m, i) => { elements[m.index] = result.elements[i] ?? null; });
          inspection = { ...result, elements };
        }
      }
      if (moved()) { flashFooter('× The conversation changed while the page was read, so nothing was added. The marks are kept.', 5000); return; }
      // C5 review: a message holds 10 attachments. With no room for the
      // picture and its page text, nothing is added and the marks stay.
      if (chatAttachments().length + (page ? 2 : 1) > ATTACH_MAX_COUNT) {
        flashFooter(`× A message holds up to ${ATTACH_MAX_COUNT} attachments. Remove one, then send again; the marks are kept.`, 5000);
        return;
      }
      // Refused (the composer says why): the marks are kept.
      if (!attachChatImage(file)) return;
      if (page) attachSharedPage(page);
      appendToChatDraft(annotationText(props.target, marks(), inspection, captureSentence(props.target.capturedAt, props.target.view)));
      done();
      props.onClose();
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`);
    } finally { setSending(false); }
  }

  /** Save to workspace: the capture and its marks become a review file in
   *  outputs/reviews/ (Rust workspace_reviews.rs). */
  async function saveToWorkspace() {
    const ws = props.workspaceId;
    if (!ws || !size() || saving()) return;
    setSaving(true);
    setEditing(null);
    try {
      const png = props.target.image.startsWith('data:image/png') ? props.target.image : await reencode(props.target.image, { type: 'image/png' });
      const set = await api.saveAnnotationSet(ws, {
        capture: { url: props.target.url, title: props.target.title, capturedAt: props.target.capturedAt, viewport: props.target.view, imageSize: size()!, marks: marks(), pngBase64: dataUrlBody(png) },
      });
      done();
      flashFooter(`Saved to Results: ${set.file}`, 4000);
      props.onSaved?.(set);
    } catch (e) {
      flashFooter(`× ${errMsg(e)}`);
    } finally { setSaving(false); }
  }

  const TOOLS: { value: Tool; label: string; hint: string }[] = [
    { value: 'pin', label: 'Comment', hint: 'Click where something needs changing' },
    { value: 'box', label: 'Box', hint: 'Drag around an area' },
    { value: 'draw', label: 'Draw', hint: 'Draw freehand' },
  ];

  return (
    <div class="browser-annotate" role="region" aria-label="Annotate page">
      <div class="annotate-bar">
        <Segmented label="Annotation tool" items={TOOLS} value={tool()} onChange={value => { cancelPending(); setTool(value); }} />
        <button type="button" class="btn small ghost" disabled={!marks().length} onClick={undo} title="Undo (⌘Z)">Undo</button>
        <button type="button" class="btn small ghost" disabled={!removed().length} onClick={redo} title="Redo (⇧⌘Z)">Redo</button>
        <span class="annotate-capture" title={`A snapshot of ${props.target.url}, not the live page`}><Icon name="clock" size={11} />{label()}</span>
        {/* One group, right-aligned: it wraps to a second line as a whole. */}
        <span class="annotate-actions">
          <Show when={marks().length}>
            <button type="button" class="btn small ghost" onClick={() => void discard()} title="Delete these marks">Discard</button>
          </Show>
          <button type="button" class="btn small ghost" onClick={props.onClose} title={marks().length ? 'Close (Esc). The marks are kept as a draft.' : 'Close (Esc)'}>Close</button>
          <Show when={props.workspaceId}>
            <button type="button" class="btn small" disabled={!size() || saving()} onClick={() => void saveToWorkspace()} title="Save the capture and its marks as a review in this workspace's Results">
              {saving() ? 'Saving…' : 'Save to workspace'}
            </button>
          </Show>
          <label class="annotate-include" title="Also add this page's readable text: its title, address, main text and tables, at most 24 KB, from the page on show now. It goes only with the message you send.">
            <input type="checkbox" checked={includeText()} onChange={e => setIncludeText(e.currentTarget.checked)} />
            Include page text
          </label>
          <button type="button" class="btn small primary" disabled={!size() || sending()} onClick={() => void send()}>
            {sending() ? 'Adding…' : 'Add to chat'}
          </button>
        </span>
      </div>
      <div ref={viewport} class="annotate-stage">
        <p class="annotate-instructions" id={instructionsId}>
          {tool() === 'draw' ? 'Draw uses a pointer. Choose Comment or Box to mark with the keyboard.'
            : 'Focus the capture. Arrows move 8 pixels; Shift + arrows move 40. Enter places a comment or sets each box corner. Escape cancels a pending box.'}
        </p>
        <button type="button" class="btn small" disabled={!size()} onClick={() => { cancelPending(); add({ kind: 'page', note: '' }, true); }}>Comment on the whole page</button>
        <Show when={size()} fallback={<div class="browser-placeholder">Capturing the page…</div>}>{s =>
          <div ref={stage} class="annotate-canvas" style={{ 'aspect-ratio': `${s().width} / ${s().height}` }} data-tool={tool()}
            role="application" tabIndex={0} aria-label="Annotation stage" aria-describedby={instructionsId} on:keydown={stageKey}
            onFocus={e => { if (e.target === e.currentTarget && e.currentTarget.matches(':focus-visible')) enterStage(); }} onBlur={() => setStageFocused(false)}>
            <img src={props.target.image} alt={`Snapshot of ${props.target.title || props.target.url}`} draggable={false} />
            <span class="annotate-capture-badge" aria-hidden="true">{label()}</span>
            <svg ref={svg} viewBox={`0 0 ${s().width} ${s().height}`} preserveAspectRatio="none" aria-label="Marks"
              onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
              <Index each={[...marks(), ...(draft() ? [draft()!] : [])]}>{mark => <>{() => {
                // A function child, so the mark being dragged redraws.
                const m = mark();
                return m.kind === 'box' ? <rect class="annotate-ink" x={m.x} y={m.y} width={m.w} height={m.h} />
                  : m.kind === 'draw' ? <path class="annotate-ink" d={path(m)} /> : null;
              }}</>}</Index>
              <Show when={stageFocused() && tool() !== 'draw' && cursor()}>{p =>
                <g class="annotate-crosshair" aria-hidden="true">
                  <path d={`M${p()[0]} 0V${s().height} M0 ${p()[1]}H${s().width}`} />
                </g>
              }</Show>
            </svg>
            {/* Index, not For: a note edit replaces its mark object, and For
                would remount the row (and drop the input's focus) per key. */}
            <Index each={marks()}>{(m, i) =>
              <Show when={m().kind !== 'page' ? m() as SpatialMark : null}>{spatial =>
              <div class="annotate-label" style={labelAt(spatial())} data-kind={m().kind}>
                <button type="button" class="annotate-num" aria-label={`Edit mark ${i + 1} on capture`} title={m().note || 'Add a comment'}
                  onPointerDown={e => e.stopPropagation()} onClick={() => setEditing(i)}>{i + 1}</button>
              </div>
              }</Show>
            }</Index>
          </div>
        }</Show>
        <p class="annotate-position" role="status" aria-live="polite" aria-atomic="true">
          {cursor() && stageFocused() ? `Position ${Math.round(cursor()![0])}, ${Math.round(cursor()![1])}. ${corner() ? 'First corner set. Move to the opposite corner, then press Enter.' : ''}` : ''}
        </p>
        <Show when={marks().length}>
          <ol class="annotate-entries" aria-label="Placed marks">
            <Index each={marks()}>{(m, i) => <li>
              <button ref={el => { entries[i] = el; }} type="button" class="annotate-entry"
                aria-label={`Mark ${i + 1}${m().note ? `: ${m().note}` : ''}`} onClick={() => setEditing(i)} on:keydown={e => entryKey(e, i)}>
                <span>{i + 1}. {m().kind === 'page' ? 'Whole page' : m().kind === 'box' ? 'Box' : m().kind === 'draw' ? 'Drawing' : 'Comment'}</span>
                <span>{m().note || 'Add a comment'}</span>
              </button>
              <Show when={editing() === i}>
                <form class="annotate-note" onSubmit={e => { e.preventDefault(); finishEditing(i); }}>
                  <input type="text" aria-label={`Comment for mark ${i + 1}`} placeholder="What should change here?" value={m().note}
                    ref={el => queueMicrotask(() => el.focus())} onInput={e => setNote(i, e.currentTarget.value)}
                    on:keydown={e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); finishEditing(i); } }} />
                  <button type="submit" class="btn small">Done</button>
                  <button type="button" class="btn small ghost" aria-label={`Remove mark ${i + 1}`} onClick={() => remove(i)}>Remove</button>
                </form>
              </Show>
            </li>}</Index>
          </ol>
        </Show>
      </div>
    </div>
  );
}
