import { For, Match, Show, Switch, Suspense, lazy, createEffect, createMemo, createResource, createRoot, createSignal, createUniqueId, onCleanup, untrack, type Resource } from 'solid-js';
import { CanvasPreparation } from './ChatMotion';
import { HtmlArtifact, type AskBridge } from './HtmlArtifact';
import { CanvasPinButton } from './CanvasPinButton';
import { canvasGrantKey } from '../canvas/canvasGrants';
import { CodeBlock } from './CodeBlock';
import { Icon } from '../icons';
import { api } from '../api';
import {
  CANVAS_FENCE_MODE, canvasFixRequest, canvasHeads, canvasKey, canvasStateSaver, chatCanvasView, hasStore, headOf, innerOf,
  resolveCanvas, saveCanvasStateCas, settleCanvasBody, versionLabel,
  type ResolvedCanvas,
} from '../canvas/chatCanvas';
import { escapeRowTags } from '../canvas/strayTags';
import { askSendLabel } from '../canvas/askSend';
import { askHold, holdKey, setAskHold } from '../canvas/askHold';
import type { CanvasState, DesignMessage, DraftRefusal } from '../canvasBridge';
import { answerKey, answeredCanvases, canvasAnswerText } from '../canvas/askAnswer';
import type { CanvasRevision } from '../generated/ipc';
import type { CannedRequest } from '../types';
import { canvasFrameBody, type CanvasRoot } from '../canvas/fence';
import { KIT_VERSION } from '../canvas/kit/manifest';
import { KIT_ASK_FIELDS } from '../canvas/kit/ask/manifest';
import { canvasVisualKey } from '../chatVisuals';
import { chatBusy, chatSessionLoading, chatTurns, chatVisuals, showChatVisual } from '../store/chat';
import { conversationCanvasScope, visitWorkspace } from '../store/workspaces';
import { canLeaveConversation } from '../store';
import { openCanvasGallery } from '../store/canvasGallery';
import { isOlder, olderLabel, savedWords, type CanvasStoreKind } from '../canvas/hostContract';
import { QUICK_CHATS_ID } from '../workspaceTypes';
import type { ActionMenuItem } from './ActionMenu';
import '../styles/chat-canvas.css';
import { plainError } from '../plainErrors';
const CanvasLayoutAudit = lazy(() => import('./CanvasLayoutAudit'));

/**
 * A `hollow-canvas` fence in a reply (Canvas C3, plan §4): nothing extra
 * while the root tag streams, a "Drawing <title>…" skeleton at the root's
 * height once it has closed, the sandboxed canvas (Canvas mode by default)
 * once the fence ends, and its source plus "Ask to fix" when it is broken.
 *
 * `live`: the reply is streaming now (or was, in this mount), so a ready
 * canvas is saved to the conversation's canvas store; a transcript re-render
 * only reads it (`resolveCanvas`). `settled`: the reply is no longer
 * streaming, so a fence that never closed reads as ended (`settleCanvasBody`)
 * and a truncated canvas shows as broken, not as a skeleton forever.
 * `inPanel`: the side panel's full-height copy (Expand), which never
 * records a revision.
 *
 * Revisions (wave 2 C2): the head shows the canvas's version ("v3"; "v2 of
 * 3" on a message whose fence is an earlier one, which also says so). With
 * more than one kept revision the label opens the version list; picking
 * another renders that revision read-only from the store (`.revisions/`).
 * State: the frame gets the canvas's saved state on load (bridge `init`), and
 * the state it posts while the person uses it is saved, debounced, with the
 * store's compare-and-swap, but only from the canvas's current revision.
 *
 * Ask-canvases (C4 A2; question cards, 2026-10-14): Hollow's own Send
 * under the frame collects the answers and `onAnswer` sends them as the
 * person's next message (`canvasAnswerText`), only from the canvas's
 * current revision. Once the person sends that message the canvas revision is answered
 * (`answeredCanvases`, read from the transcript): the frame locks and shows
 * the answer, and the host says so under it. A settled, unanswered current
 * ask-canvas says it is waiting on the person.
 */

/** What an ask-canvas's draft asks the composer for (C4 A2). */
export interface CanvasDraftRequest {
  canvasId: string;
  title: string;
  /** The composer text: `canvasAnswerText`. */
  text: string;
  /** The revision was answered meanwhile: never fill. */
  locked: () => boolean;
  /** Tell the frame `confirm` while the person chooses. */
  interim: (r: DraftRefusal) => void;
  /** The question over the composer when it holds the person's own text
   *  (default: the ask-canvas's "Answers from … are ready"). Canvas C6 · K2:
   *  a curated view's Refresh asks with its own words. */
  confirm?: string;
  /** Hollow wrote `text` (a curated Refresh, overnight 9 A1): sent
   *  unchanged, it goes with this tag. An ask-canvas's answers have none. */
  canned?: CannedRequest;
}
export type CanvasDraftHandler = (req: CanvasDraftRequest) => Promise<DraftRefusal | null>;

/** An ask-canvas's answers to send as the person's next message (question
 *  cards, 2026-10-14). */
export interface CanvasAnswerRequest {
  canvasId: string;
  title: string;
  /** The message: `canvasAnswerText`. */
  text: string;
  /** The revision was answered meanwhile: never send. */
  locked: () => boolean;
  /** The conversation whose canvas this is (its store's key): the answers
   *  go only there (a side-panel copy outlives a switch). */
  sessionKey: string | null;
}
/** Sent (null), held for after the running turn (`held`: the held send's
 *  id, P1-33), or why not. */
export type CanvasAnswerResult = DraftRefusal | null | { held: string };
export type CanvasAnswerHandler = (req: CanvasAnswerRequest) => Promise<CanvasAnswerResult>;

// The conversation a canvas outside the transcript (the side panel's
// Expand copy) sends its answers to: the chat that registered last.
const [panelAnswer, setPanelAnswer] = createSignal<CanvasAnswerHandler | null>(null);

/** A chat takes answers from canvases outside its transcript (the side
 *  panel) while it is mounted. Call from the chat's setup. */
export function registerCanvasAnswers(handler: CanvasAnswerHandler) {
  setPanelAnswer(() => handler);
  onCleanup(() => { if (panelAnswer() === handler) setPanelAnswer(null); });
}

// The composer a canvas outside the transcript (the side panel's Expand
// copy) fills: the chat that registered last, while it is mounted.
const [composerDraft, setComposerDraft] = createSignal<CanvasDraftHandler | null>(null);

/** A chat's composer takes drafts from canvases outside its transcript
 *  (the side panel) while it is mounted. Call from the chat's setup. */
export function registerCanvasComposer(handler: CanvasDraftHandler) {
  setComposerDraft(() => handler);
  onCleanup(() => { if (composerDraft() === handler) setComposerDraft(null); });
}

/** The mounted chat composer's fill (null with none on screen). It fills and
 *  never sends, never moves focus, and replaces the person's own text only on
 *  their Replace. (An ask-canvas's Send used it until question cards,
 *  2026-10-14; it now sends through `registerCanvasAnswers`.) */
export function composerFill(): CanvasDraftHandler | null {
  return composerDraft();
}
export function ChatCanvas(props: {
  code: string;
  closed: boolean;
  settled?: boolean;
  live?: boolean;
  onAsk?: (text: string) => void;
  onAnswer?: CanvasAnswerHandler;
  inPanel?: boolean;
  compact?: boolean;
  /** The history run id of the reply this fence is in (`resolveCanvas`). */
  turnId?: string | null;
}) {
  // A stray `<` in a data row is text (overnight A2): escaped before the
  // body is parsed, saved or drawn, so the store keeps the escaped body.
  const body = createMemo(() => settleCanvasBody(escapeRowTags(props.code), props.closed, !!props.settled), undefined, {
    equals: (a, b) => a.code === b.code && a.closed === b.closed,
  });
  /** The body as the agent wrote it, when escaping changed it. */
  const written = () => { const raw = settleCanvasBody(props.code, props.closed, !!props.settled).code; return raw === body().code ? null : raw; };
  const view = createMemo(() => chatCanvasView(body().code, body().closed));
  const drawing = () => { const v = view(); return v.state === 'drawing' ? v : null; };
  const broken = () => { const v = view(); return v.state === 'invalid' ? v : null; };
  const ready = () => { const v = view(); return v.state === 'ready' ? v : null; };
  return (
    <Switch>
      <Match when={drawing()}>{(v) => <CanvasSkeleton title={v().root.title} height={v().reserve} />}</Match>
      <Match when={broken()}>{(v) => <BrokenCanvas root={v().root} reason={v().reason} source={body().code} onAsk={props.onAsk} />}</Match>
      <Match when={ready()} keyed>
        {(v) => <ReadyCanvas root={v.root} inner={v.inner} code={body().code} written={untrack(written)} live={untrack(() => !!props.live)} onAsk={props.onAsk} onAnswer={props.onAnswer} settled={!!props.settled} inPanel={props.inPanel} compact={props.compact} turnId={untrack(() => props.turnId ?? null)} />}
      </Match>
    </Switch>
  );
}

function CanvasSkeleton(props: { title: string; height: number }) {
  return <CanvasPreparation title={props.title} height={props.height} />;
}

function BrokenCanvas(props: { root: Pick<CanvasRoot, 'id' | 'title'> | null; reason: string; source: string; onAsk?: (text: string) => void; runtime?: boolean }) {
  const name = () => props.root?.title || props.root?.id || '';
  return (
    <div class="chat-canvas-broken" role="group" aria-label={name() ? `Broken canvas: ${name()}` : 'Broken canvas'}>
      <div class="chat-canvas-broken-head">
        <Icon name="alert" size={14} />
        <span>{props.runtime ? 'This canvas reported an error' : 'This canvas couldn’t be shown'}{name() ? <>: <strong>{name()}</strong></> : null}</span>
      </div>
      <p class="chat-canvas-broken-reason">{props.reason}</p>
      <Show when={!props.runtime} fallback={
        <details class="chat-canvas-source"><summary>Source</summary><CodeBlock code={props.source} lang="hollow-canvas" /></details>
      }>
        <CodeBlock code={props.source} lang="hollow-canvas" />
      </Show>
      <Show when={props.onAsk}>
        {/* Drafts the request in the composer; the person presses Enter. */}
        <button type="button" class="btn small chat-canvas-fix" title="Draft a fix request in the composer (not sent)"
          onClick={() => props.onAsk?.(canvasFixRequest(props.root, props.reason))}>
          <Icon name="edit" size={13} />Ask to fix
        </button>
      </Show>
    </div>
  );
}

// Re-render canvases when this window learns a newer head (a later message
// revised the canvas, a state save landed).
const [headsTick, setHeadsTick] = createSignal(0);
canvasHeads.subscribe(() => setHeadsTick(t => t + 1));

/** A resource's value once it has one (the last one while it refreshes),
 *  read without suspending. */
function settled<T>(r: Resource<T>): T | undefined {
  return r.state === 'ready' || r.state === 'refreshing' ? r.latest : undefined;
}

/**
 * Give focus back to `el` when a re-render drops it (C6 P2). Picking a
 * version, "Back to vN" and Escape in the version list remove or move the
 * focused control (the version label moves between the two frames' heads
 * once the revision has loaded), and focus fell to the page. For a short
 * while, whenever focus is nowhere, it goes to `el`; anything the person
 * focuses meanwhile wins.
 */
export function keepFocus(el: HTMLElement | null | undefined, ms = 1500) {
  if (!el) return;
  const until = performance.now() + ms;
  const step = () => {
    const a = document.activeElement;
    if (!a || a === document.body) { if (el.isConnected) el.focus(); }
    else if (a !== el) return; // the person moved on
    if (performance.now() < until) requestAnimationFrame(step);
  };
  queueMicrotask(step);
}

/** `st` without the answers ask fields keep (`hc:<field tag>…` keys, P1-33);
 *  the same object when it holds none. An answered canvas saves none, so a
 *  later revision never opens on them. */
function withoutAnswers(st: CanvasState): CanvasState {
  const kept = st ? Object.keys(st).filter(k => KIT_ASK_FIELDS.some(t => k.startsWith(`hc:${t}:`) || k.startsWith(`hc:${t}#`))) : [];
  if (!st || !kept.length) return st;
  const rest = { ...st };
  for (const k of kept) delete rest[k];
  return rest;
}

const unsaved = (body: string): ResolvedCanvas => ({ body, revision: null, latest: null, saved: false, note: null, canvas: null });

// The ask-canvas revisions this conversation has answered (C4 A2): read from
// its transcript, so a lock lasts as long as the message that made it.
const answered = createRoot(() => createMemo(() => answeredCanvases(chatTurns)));

function ReadyCanvas(props: {
  root: CanvasRoot; inner: string; code: string; written?: string | null; live: boolean; settled: boolean;
  onAsk?: (text: string) => void; onAnswer?: CanvasAnswerHandler; inPanel?: boolean; compact?: boolean;
  turnId?: string | null;
}) {
  // The store is the conversation's at mount time; a canvas never moves stores.
  const scope = conversationCanvasScope();
  const storeId = props.root.id;
  const key = hasStore(scope) ? canvasKey(scope, storeId) : null;
  const [savedRes] = createResource(
    () => props.code,
    async (body): Promise<ResolvedCanvas> => {
      const r = await resolveCanvas(api, scope, storeId, body, { live: props.live && !props.inPanel, kit: KIT_VERSION, turnId: props.turnId ?? null, written: props.written ?? null }).catch(() => unsaved(body));
      if (key && r.canvas) canvasHeads.note(key, headOf(r.canvas));
      return r;
    },
  );
  // Never read a pending resource here: the transcript sits inside a
  // Suspense boundary, and a pending read swaps the whole conversation for
  // its fallback for that moment, which throws the person's scroll back to
  // the top (and, with the frame budget, parks every frame below it).
  const saved = () => settled(savedRes);
  const head = () => { headsTick(); return key ? canvasHeads.get(key) : undefined; };
  /** This fence's revision, and the store's current one. */
  const own = () => saved()?.revision ?? null;
  const latest = () => Math.max(head()?.revision ?? 0, saved()?.latest ?? 0) || null;
  const current = () => !!saved()?.saved && own() !== null && own() === latest();
  const revisions = (): CanvasRevision[] => head()?.revisions ?? saved()?.canvas?.revisions ?? [];
  const state = (): CanvasState => head()?.state ?? null;

  // A chosen earlier (or, from an older message, the current) revision,
  // read from the store and shown read-only.
  const [viewing, setViewing] = createSignal<number | null>(null);
  const [otherRes] = createResource(
    () => { const n = viewing(); return n !== null && hasStore(scope) ? n : false; },
    async (n): Promise<{ n: number; body: string; inner: string } | { n: number; error: string }> => {
      try {
        const r = await api.readCanvasRevision(scope!, storeId, n);
        return { n, body: r.html, inner: innerOf(r.html, '') };
      } catch (e) {
        return { n, error: e instanceof Error ? e.message : String(e) };
      }
    },
  );
  const other = () => settled(otherRes);
  const shownOther = () => { const o = other(); return o && 'body' in o && o.n === viewing() ? o : null; };
  const otherError = () => { const o = other(); return o && 'error' in o && o.n === viewing() ? o.error : null; };

  const canSaveState = () => latest() !== null && (viewing() === null ? current() : shownOther()?.n === latest());
  const [saveStatus, setSaveStatus] = createSignal<'pending' | 'saving' | 'failed' | 'saved'>('saved');
  let saverDisposed = false;
  const saver = canvasStateSaver((st, at, revision) => (hasStore(scope) && revision === latest()
    ? saveCanvasStateCas(api, canvasHeads, scope, storeId, st, at, revision) : Promise.resolve('kept-newer')), 400,
  status => { if (!saverDisposed) setSaveStatus(status); });
  onCleanup(() => { saverDisposed = true; void saver.flush(); });
  const onState = (st: CanvasState) => { if (canSaveState()) saver.push(lock() ? withoutAnswers(st) : st, Date.now(), latest()!); };

  // Ask-canvas (C4 A2): this fence's revision locks once its answer is sent.
  const onAnswer = () => props.onAnswer ?? (props.inPanel ? panelAnswer() : null);
  const lock = () => { const n = own(); return n === null ? undefined : answered().get(answerKey(props.root.id, n)); };
  // Answers held for after the running turn, or a Send that didn't go
  // (P1-33): one hold per conversation and revision, so the transcript's
  // copy and the side panel's show the same and send once.
  const hk = () => { const n = own(); return n === null ? null : holdKey(scope.sessionKey ?? null, answerKey(props.root.id, n)); };
  const hold = () => { const k = hk(); return k ? askHold(k) : null; };
  // `sending` is set before the send starts, so the other copy's Send is off
  // from that moment (the send controller's preflight can take a while).
  const deliver = (text: string): Promise<DraftRefusal | null> => {
    const k = hk(), before = k ? askHold(k) : null, handler = onAnswer();
    if (before?.kind === 'sending' || before?.kind === 'queued') return Promise.resolve('busy');
    if (!handler) return Promise.resolve('unavailable');
    if (k) setAskHold(k, { kind: 'sending' });
    return handler({
      canvasId: props.root.id, title: props.root.title, text,
      locked: () => !!lock(),
      sessionKey: scope.sessionKey ?? null,
    }).catch((): CanvasAnswerResult => 'not-sent').then((r) => {
      if (r !== null && typeof r === 'object') { if (k) setAskHold(k, { kind: 'queued', held: r.held }); return null; }
      if (k) setAskHold(k, r === 'not-sent' ? { kind: 'not-sent', text } : r === null || r === 'locked' ? null : before);
      return r;
    });
  };
  // An earlier revision (P1-33) has no Send: Go to vN brings the current
  // one's reply into view and focuses its Send (or its callout's first
  // button). False when that reply isn't in this transcript: the button's
  // line then says how to answer.
  const goNewer = (): boolean => {
    const n = latest();
    const there = n === null ? undefined : Array.from(document.querySelectorAll<HTMLElement>(`.turn .chat-canvas[data-revision="${n}"]`))
      .find(el => el !== root && el.dataset.canvasId === props.root.id);
    if (!there) return false;
    // The button, not the fence: a tall canvas centred leaves Send below.
    const target = there.querySelector<HTMLElement>('.chat-ask-send-btn') ?? there.querySelector<HTMLElement>('.chat-ask-send button');
    (target ?? there).scrollIntoView({ block: 'center' });
    target?.focus({ preventScroll: true });
    return true;
  };
  // Answered (P1-33): the answers the fields kept while the person chose
  // (`hc:<field tag>…` keys) leave the saved state; the lock shows them now.
  // Only a lock that lands while this copy shows the revision open: opening
  // a canvas answered earlier never writes.
  // A save still waiting in the saver's debounce goes first, so it can't
  // write the answers back afterwards.
  let open = false;
  createEffect(() => {
    if (own() !== null && !lock()) { open = true; return; }
    if (!open || !lock()) return;
    open = false;
    void saver.flush().then(() => {
      const st = untrack(state);
      if (!st || !untrack(canSaveState)) return;
      const rest = withoutAnswers(st);
      if (rest !== st) saver.push(rest, Date.now(), untrack(latest)!);
    });
  });
  const askBridge: AskBridge | undefined = props.root.ask ? {
    answered: () => lock()?.summary ?? null,
    refusal: () => (!current() ? 'not-current' : !onAnswer() ? 'unavailable' : null),
    send: (d) => deliver(canvasAnswerText({ canvasId: props.root.id, revision: own()!, answers: d.answers }, d.summary)),
    label: askSendLabel(props.inner),
    hold,
    release: () => { const k = hk(); if (k) setAskHold(k, null); },
    // Not while this revision's own send is going: its preflight sets busy too.
    queues: () => chatBusy() && !chatSessionLoading() && hold()?.kind !== 'sending',
    newer: () => (viewing() === null && isOlder(own(), latest()) ? { revision: latest()!, go: goNewer } : null),
  } : undefined;
  // An earlier revision shown read-only never sends.
  const pastAsk: AskBridge | undefined = props.root.ask
    ? { answered: () => null, refusal: () => 'not-current', send: () => Promise.resolve('not-current'), label: '', past: true }
    : undefined;
  const waiting = () => props.root.ask && props.settled && current() && !lock() && viewing() === null;
  // What a version change did, said in a live region that stays mounted
  // (the note under the frame is drawn anew, so it isn't announced). Empty
  // until the person picks a version: mounting says nothing.
  const [picked, setPicked] = createSignal(false);
  const said = () => {
    if (!picked()) return '';
    const n = viewing();
    if (n === null) return own() === null ? '' : `Showing v${own()}, this reply’s version.`;
    if (otherError()) return `Couldn’t open v${n}.`;
    if (!shownOther()) return `Opening v${n}…`;
    return n === latest() ? `Showing v${n}, the current version.` : `Showing v${n}, an earlier version, read-only.`;
  };
  let root: HTMLDivElement | undefined;
  const versionButton = () => root?.querySelector<HTMLElement>('button.chat-artifact-version');
  const pick = (n: number | null) => { setPicked(true); setViewing(n); keepFocus(versionButton()); };

  const [runtimeError, setRuntimeError] = createSignal<string | null>(null);
  // Keyed by the fence as written, as `chatVisuals` keys it.
  const visualKey = canvasVisualKey(props.written ?? props.code);
  // Expand opens it in the Preview pane, beside any page (run 15 P1).
  const expandable = () => !props.inPanel && chatVisuals().some(v => v.key === visualKey);
  // The host contract (plan L3 X7/X8, 2026-10-14; review U canvases): the
  // saved state and an older version in words, and Compare in the gallery.
  const storeKind: CanvasStoreKind = !hasStore(scope) ? null
    : scope.workspaceId ? (scope.workspaceId === QUICK_CHATS_ID ? 'quick' : 'workspace') : 'conversation';
  const savedState = () => {
    const s = saved(), shown = shownOther();
    return s ? savedWords({ store: storeKind, saved: !!shown || s.saved, revision: shown?.n ?? s.revision, latest: latest() }) : null;
  };
  const older = () => { const n = viewing() ?? own(), l = latest(); return isOlder(n, l) ? olderLabel(n!, l!) : null; };
  const compare = (): ActionMenuItem[] => {
    const ws = scope.workspaceId;
    if (!ws || revisions().length < 2) return [];
    const session = scope.sessionKey ?? undefined;
    return [{
      label: 'Compare versions in the gallery', disabled: !canLeaveConversation(),
      action: () => {
        void openCanvasGallery(ws, { workspaceId: ws, id: storeId, compare: true, back: { label: 'Back to the conversation', go: () => void visitWorkspace(ws, session) } });
      },
    }];
  };
  const host = props.inPanel ? 'expanded' as const : 'inline' as const;
  // The findings row (flow layout, and since repo ideas 2026-10-27 #1 the
  // design check): a settled reply's current canvas. Its frame is asked for
  // one design report then (`onDesign`), never while the reply streams.
  const audited = () => (props.settled || (props.inPanel && !chatBusy())) && current() && viewing() === null && !!props.onAsk;
  const [design, setDesign] = createSignal<DesignMessage | null>(null);
  // Panel-only disclosure survives the keyed artifact swap on revision changes.
  const [controlsOpen, setControlsOpen] = createSignal(false);
  const compactControls = { open: controlsOpen, setOpen: setControlsOpen };
  // One label for whichever revision shows: created once, so its open state
  // survives switching revisions (it moves between the two frames' heads).
  const badge = (<>
    <CanvasVersions title={props.root.title} shown={viewing() ?? own()} own={own()} latest={latest()} revisions={revisions()}
      onOpen={() => { if (key && hasStore(scope)) void api.readCanvas(scope, storeId).then(c => { if (c) canvasHeads.note(key, headOf(c)); }).catch(() => {}); }}
      onPick={(n) => pick(n === own() ? null : n)} />
    <Show when={current() && viewing() === null && saved()?.canvas}>{c => <CanvasPinButton canvas={c()}/>}</Show>
  </>);
  return (
    <div class="chat-canvas" ref={root} data-canvas-id={props.root.id} data-saved={saved()?.saved ? 'true' : 'false'}
      data-revision={viewing() ?? own() ?? undefined}>
      <span class="chat-canvas-live sr-only" role="status">{said()}</span>
      <Show when={saved()} fallback={<CanvasSkeleton title={props.root.title} height={props.root.height ?? 240} />}>
        {(s) => <>
          <Show when={shownOther()} keyed fallback={
            <HtmlArtifact compact={props.compact} compactControls={props.compact ? compactControls : undefined} compactVersion={props.compact && own() !== null ? versionLabel(own()!, latest()) : null} lookWorkspace={scope.workspaceId}
              code={`\n${innerOf(s().body, canvasFrameBody({ root: props.root, inner: props.inner }))}\n`}
              defaultMode={CANVAS_FENCE_MODE}
              ask={props.root.ask}
              canvas={{ title: props.root.title, source: props.code, badge, ref: scope.workspaceId && s().saved && own() !== null ? { workspaceId: scope.workspaceId, revision: own()!, state: state() } : undefined }}
              bridge={{ canvasId: props.root.id, state, onState: canSaveState() ? onState : undefined, ask: askBridge, conversation: scope.sessionKey }}
              stateTarget={latest() ? { revision: latest()!, open: () => pick(latest() === own() ? null : latest()) } : undefined}
              grantKey={canvasGrantKey(scope, props.root.id)}
              initialHeight={props.root.height ?? undefined}
              fill={props.inPanel}
              host={host} saved={savedState()} older={older()} actions={compare()}
              onExpand={expandable() ? () => showChatVisual(visualKey) : undefined}
              onError={(m) => { if (!runtimeError()) setRuntimeError(m || 'Unknown error'); }}
              onDesign={audited() ? setDesign : undefined}
            />
          }>
            {(o) => (
              <HtmlArtifact compact={props.compact} compactControls={props.compact ? compactControls : undefined} compactVersion={props.compact && viewing() !== null ? versionLabel(viewing()!, latest()) : null} lookWorkspace={scope.workspaceId}
                code={`\n${o.inner}\n`}
                defaultMode={CANVAS_FENCE_MODE}
                ask={props.root.ask}
                canvas={{ title: props.root.title, source: o.body, badge, ref: scope.workspaceId ? { workspaceId: scope.workspaceId, revision: viewing() ?? undefined, state: state() } : undefined }}
                bridge={{ canvasId: props.root.id, state, onState: canSaveState() ? onState : undefined, ask: pastAsk, conversation: scope.sessionKey }}
                stateTarget={latest() ? { revision: latest()!, open: () => pick(latest() === own() ? null : latest()) } : undefined}
                grantKey={canvasGrantKey(scope, props.root.id)}
                initialHeight={props.root.height ?? undefined}
                fill={props.inPanel}
                host={host} saved={savedState()} older={older()} actions={compare()}
              />
            )}
          </Show>
          <Show when={saveStatus() === 'failed'}><p class="chat-canvas-note" role="status">Your changes couldn’t be saved.{' '}
            <button type="button" class="chat-canvas-back" disabled={!canSaveState()} onClick={() => void saver.flush()}>Try again</button>
          </p></Show>
          <Show when={viewing()} fallback={
            <Show when={own() !== null && latest() !== null && own()! < latest()!}>
              <p class="chat-canvas-note">An earlier version of this canvas; v{latest()} is current.</p>
            </Show>
          }>
            {(n) => (
              <p class="chat-canvas-note">
                {otherError() ? plainError(`Couldn’t open v${n()}: ${otherError()}`).text : n() === latest() ? `Showing v${n()}, the current version.` : `Showing v${n()}, an earlier version, read-only.`}{' '}
                <button type="button" class="chat-canvas-back" onClick={() => pick(null)}>Back to v{own()}</button>
              </p>
            )}
          </Show>
          <Show when={audited()}>
            <Suspense><CanvasLayoutAudit source={props.code} root={props.root} scope={scope} revision={own()} state={state()} onAsk={props.onAsk!}
              design={design()} runId={props.inPanel ? null : props.turnId ?? null} /></Suspense>
          </Show>
          <Show when={s().note}>{(n) => <p class="chat-canvas-note">{n()}</p>}</Show>
          <Show when={waiting()}>
            <p class="chat-canvas-ask-state" data-ask-state="waiting" role="status"><Icon name="clock" size={13} />Waiting on your answer</p>
          </Show>
          <Show when={lock() && viewing() === null}>
            <p class="chat-canvas-ask-state" data-ask-state="answered" role="status"><Icon name="check" size={13} />Answered. You sent this canvas’s answers.</p>
          </Show>
        </>}
      </Show>
      <Show when={runtimeError()}>
        {(err) => <BrokenCanvas runtime root={props.root} reason={err()} source={props.code} onAsk={props.onAsk} />}
      </Show>
    </div>
  );
}

/** The version label in a canvas's head. With more than one kept revision it
 *  is a disclosure button over the list of versions (newest first); picking
 *  one shows it. */
export function CanvasVersions(props: {
  title: string; shown: number | null; own: number | null; latest: number | null; revisions: CanvasRevision[];
  onOpen: () => void; onPick: (revision: number) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const listId = createUniqueId();
  const newestFirst = () => [...props.revisions].sort((a, b) => b.revision - a.revision);
  const toggle = () => { const next = !open(); setOpen(next); if (next) props.onOpen(); };
  let button: HTMLButtonElement | undefined;
  return (
    <Show when={props.shown !== null}>
    <Show when={props.revisions.length > 1} fallback={<span class="chat-artifact-version">{versionLabel(props.shown!, props.latest)}</span>}>
      <span class="chat-canvas-versions" onKeyDown={(e) => { if (e.key === 'Escape' && open()) { e.stopPropagation(); setOpen(false); keepFocus(button); } }}>
        <button ref={button} type="button" class="chat-artifact-version" aria-expanded={open()} aria-controls={listId}
          title="Show this canvas’s versions" onClick={toggle}>
          {versionLabel(props.shown!, props.latest)}<span class="chat-canvas-versions-caret" aria-hidden="true"><Icon name={open() ? 'chevronDown' : 'chevronRight'} size={12} /></span>
        </button>
        <Show when={open()}>
          <span class="chat-canvas-version-list" id={listId} role="group" aria-label={`Versions of ${props.title}`}>
            <For each={newestFirst()}>
              {(r) => (
                <button type="button" class="chat-canvas-version" aria-pressed={r.revision === props.shown}
                  title={`Saved ${new Date(r.savedAt).toLocaleString()}`}
                  onClick={() => { setOpen(false); props.onPick(r.revision); }}>
                  v{r.revision}
                  <Show when={r.revision === props.latest}><span class="chat-canvas-version-tag"> current</span></Show>
                  <Show when={r.revision === props.own && props.own !== props.latest}><span class="chat-canvas-version-tag"> this reply</span></Show>
                </button>
              )}
            </For>
          </span>
        </Show>
      </span>
    </Show>
    </Show>
  );
}
