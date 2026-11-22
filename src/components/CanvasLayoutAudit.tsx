import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import { flowsIn } from '../canvas/canvasFlows';
import { auditFlow, autoAuditFlow } from '../canvas/flowAudit';
import { DESIGN_SHOWN, TIER_WORD, canvasFindingsRequest, designFindings } from '../canvas/designAudit';
import { noteCanvasProblems, problemsLabel } from '../canvas/canvasProblems';
import { canvasLayoutRequest, type ChatCanvasScope } from '../canvas/chatCanvas';
import type { CanvasRoot } from '../canvas/fence';
import type { CanvasState, DesignMessage } from '../canvasBridge';
import { api } from '../api';
import { exportPalette } from '../canvas/exportLook';
import type { PixelVerdict } from '../canvas/pixelContrast';
import { attachChatImage, chatAgent, chatBusy, chatConversationVersion, chatInput } from '../store/chatRuntime';
import { conversationCanvasScope, workspaces } from '../store/workspaces';

/** Lazy, reply-only findings row: hc-flow layout, and since repo ideas
 *  2026-10-27 #1 the design check of the canvas's hand-written parts (its
 *  source, plus the frame's measured `design` report once it comes). One
 *  collapsed row per canvas; advisory only, it never changes or hides the
 *  canvas. Unmounting (revision/scope change) cancels attachment. */
export default function CanvasLayoutAudit(props: {
  source: string; root: CanvasRoot; scope: ChatCanvasScope; revision: number | null;
  state: CanvasState; onAsk: (text: string) => void;
  /** The frame's measured report, once it came (untrusted, bounded). */
  design?: DesignMessage | null;
  /** The reply's run, whose receipt shows the counts (null: the side panel's copy). */
  runId?: string | null;
}) {
  const flows = createMemo(() => flowsIn(props.source));
  let alive = true;
  onCleanup(() => { alive = false; });
  const [busy, setBusy] = createSignal(false), [message, setMessage] = createSignal('');
  // Large diagrams the person asked to check (by index in the source).
  const [checked, setChecked] = createSignal<ReadonlySet<number>>(new Set());
  const layout = createMemo(() => flows().map((flow, i) => {
    const automatic = autoAuditFlow(flow.text);
    return { flow, i, automatic, findings: automatic || checked().has(i) ? auditFlow(flow.text, flow.direction) : null };
  }));
  const withFindings = () => layout().filter(l => l.findings?.length);
  // Repo ideas 2026-10-27 #4: the unresolved texts measured on the picture,
  // for the report they were measured from (a new report starts over).
  const [pictures, setPictures] = createSignal<{ report: DesignMessage; verdicts: ReadonlyMap<number, PixelVerdict> } | null>(null);
  const [measuring, setMeasuring] = createSignal(false), [measureNote, setMeasureNote] = createSignal('');
  const verdicts = () => { const p = pictures(); return p && p.report === props.design ? p.verdicts : undefined; };
  const design = createMemo(() => designFindings(props.source, props.design ? { items: props.design.items, more: props.design.more, complete: props.design.complete, pictures: verdicts() } : null));
  /** Unresolved texts the picture can still settle (each needs a box). */
  const measurable = () => !!props.design && props.design.items.some((item, i) => item.rule === 'texture-text' && !!item.box && !verdicts()?.has(i));
  // A note belongs to the report it was measured from.
  createEffect(on(() => props.design, () => setMeasureNote(''), { defer: true }));
  const measure = async () => {
    const report = props.design;
    if (!report || measuring()) return;
    setMeasuring(true); setMeasureNote('');
    try {
      const themeName = workspaces().find(w => w.id === props.scope.workspaceId)?.canvasTheme;
      const { measureOnPicture } = await import('../canvas/measurePicture');
      const verdicts = await measureOnPicture(props.source, props.state, report, themeName);
      if (!alive || props.design !== report) return;
      setPictures({ report, verdicts });
      const settled = [...verdicts.values()].filter(v => v.kind === 'measured').length;
      setMeasureNote(settled ? `Measured ${settled} of ${verdicts.size} on the picture.` : 'None could be measured on the picture.');
    } catch (err) {
      if (alive && props.design === report) setMeasureNote(`Couldn't measure on the picture: ${err instanceof Error ? err.message : String(err)}`);
    } finally { if (alive) setMeasuring(false); }
  };
  const layoutCount = () => withFindings().reduce((n, l) => n + l.findings!.filter(f => f.kind !== 'more').length, 0);
  const layoutMore = () => withFindings().some(l => l.findings!.some(f => f.kind === 'more'));
  const summary = () => {
    const only = withFindings();
    if (!design().total && only.length === 1) {
      const f = only[0].findings!;
      return `${f.filter(x => x.kind !== 'more').length}${f.some(x => x.kind === 'more') ? '+' : ''} layout problem${f.length === 1 ? '' : 's'} in "${only[0].flow.label}"`;
    }
    return problemsLabel(layoutCount(), design().total, layoutMore()) ?? '';
  };
  // The reply's receipt says the same count in its Canvases row, while this
  // row is on screen: an earlier revision's row or an unmounted transcript
  // takes its count with it.
  const owner = {};
  createEffect(() => {
    const run = props.runId, id = props.root.id;
    if (!run) return;
    noteCanvasProblems(run, id, { layout: layoutCount(), design: design().total, more: layoutMore() }, owner);
    onCleanup(() => noteCanvasProblems(run, id, null, owner));
  });
  const here = () => {
    const now = conversationCanvasScope();
    return now.workspaceId === props.scope.workspaceId && now.sessionKey === props.scope.sessionKey;
  };
  const fix = async () => {
    if (!here() || busy()) return;
    const only = withFindings();
    props.onAsk(!design().total && only.length === 1
      ? canvasLayoutRequest(props.root, only[0].flow.label, only[0].findings!)
      : canvasFindingsRequest(props.root, only.map(l => ({ label: l.flow.label, findings: l.findings! })), design()));
    const agent = chatAgent();
    // No image capability is advertised by AgentDescriptor yet. These two
    // installed CLI paths support local image reading; others get text only.
    if (agent !== 'claude' && agent !== 'codex') return;
    const version = chatConversationVersion(), draft = chatInput();
    const valid = () => alive && here() && chatConversationVersion() === version && chatAgent() === agent && chatInput() === draft && !chatBusy();
    if (!valid()) return;
    setBusy(true); setMessage('');
    const themeName = workspaces().find(w => w.id === props.scope.workspaceId)?.canvasTheme;
    let paletteReady = false;
    try {
      if (!props.scope.workspaceId) throw new Error('No workspace capture target');
      const palette = await exportPalette(themeName);
      paletteReady = true;
      if (!valid()) return;
      const result = await api.canvasExport({ workspaceId: props.scope.workspaceId, revision: props.revision ?? undefined,
        source: props.source, state: props.state, format: 'png', target: 'attachment', preset: 'doc', look: 'match',
        copy: false, ...palette });
      if (!valid()) return;
      if (!result.pngBase64) throw new Error('No picture returned');
      const bytes = Uint8Array.from(atob(result.pngBase64), c => c.charCodeAt(0));
      // Never leave an asynchronous capture in the global pending queue:
      // the next composer could belong to a different conversation.
      if (!attachChatImage(new File([bytes], `${props.root.id}-layout.png`, { type: 'image/png' }), false)) throw new Error('Composer closed');
    } catch {
      if (valid()) setMessage(!paletteReady && themeName
        ? `Couldn't read canvas look “${themeName}”; the request lists the findings without a picture.`
        : "Couldn't attach a picture; the request lists the findings.");
    } finally { if (alive) setBusy(false); }
  };
  const several = () => withFindings().length + (design().total ? 1 : 0) > 1;
  return <>
    <For each={layout()}>{l => <>
      <Show when={!l.automatic && !checked().has(l.i)}><p class="chat-canvas-note">Large diagram "{l.flow.label}". <button class="btn small" onClick={() => setChecked(s => new Set(s).add(l.i))}>Check layout</button></p></Show>
      <Show when={checked().has(l.i) && l.findings?.length === 0}><p class="chat-canvas-note">No layout problems found in "{l.flow.label}".</p></Show>
    </>}</For>
    <Show when={withFindings().length || design().total}>
      <details class="chat-canvas-layout">
        <summary>{summary()}</summary>
        <For each={withFindings()}>{l => <>
          <Show when={several()}><p class="chat-canvas-layout-head">Diagram "{l.flow.label}"</p></Show>
          <p>Estimated for the authored direction; the displayed layout may differ.</p>
          <ol><For each={l.findings!}>{f => <li>{f.text}</li>}</For></ol>
        </>}</For>
        <Show when={design().total}>
          <p class="chat-canvas-layout-head">Hand-written HTML and CSS</p>
          <p>Advisory: the canvas is shown as written.</p>
          <ol class="chat-canvas-design"><For each={design().list.slice(0, DESIGN_SHOWN)}>{f => (
            <li data-tier={f.tier}><strong>{TIER_WORD[f.tier]}</strong>: {f.text} <span class="chat-canvas-fix">{f.fix}</span></li>
          )}</For></ol>
          <Show when={design().total - Math.min(design().list.length, DESIGN_SHOWN)}>{rest => <p>And {rest()} more design problem{rest() === 1 ? '' : 's'}.</p>}</Show>
          {/* Repo ideas 2026-10-27 #4: on demand, four offscreen captures. */}
          <Show when={measurable()}>
            <button class="btn small" disabled={measuring()} title="Draws this canvas four times offscreen (its text as it is, hidden, black and white) and scores each unresolved text against what lies under it. Nothing is saved or sent."
              onClick={() => void measure()}>{measuring() ? 'Measuring…' : 'Measure on the picture'}</button>
          </Show>
          <Show when={measureNote()}><p role="status">{measureNote()}</p></Show>
        </Show>
        <button class="btn small" disabled={busy()} title="Draft a correction; nothing is sent" onClick={() => void fix()}>Ask to fix</button>
      </details>
    </Show>
    <Show when={message()}><p class="chat-canvas-note" role="status">{message()}</p></Show>
  </>;
}
