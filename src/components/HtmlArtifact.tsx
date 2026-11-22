import { CanvasPreparation } from './ChatMotion';
import { motionReduced } from '../motion';
import { untrack } from 'solid-js';
import { lookFor } from '../store/canvasLook';
import type { Look } from '../canvasLook';
import { batch, createEffect, on, createSignal, createMemo, createResource, createUniqueId, onMount, onCleanup, For, Match, Show, Switch, useContext, type JSX } from 'solid-js';
import { flashFooter } from '../store';
import {
  DEFAULT_ARTIFACT_MODE, SANDBOX,
  NO_KIT, artifactDocUrl, loadCanvasKit, loadCanvasKitAsk, loadedCanvasKit, loadedCanvasKitAsk, modeRunsScripts,
  sandboxDocParts, sealDoc, srcdocMode, watchFrameTheme,
  type ArtifactMode,
} from './htmlSandbox';
import type { CanvasKitAssets } from '../canvas/kitAssets';
import {
  COLLECT_TIMEOUT_MS, acceptFrameMessage, answeredMessage, collectMessage, designCheckMessage, draftRefusal, initMessage, newCollectNonce,
  type CanvasState, type DesignMessage, type DraftAnswers, type DraftMessage, type DraftRefusal,
} from '../canvasBridge';
import { askRefusalStatus, MOVED_STATUS, NOT_SENT_STATUS, QUEUE_LABEL, QUEUED_STATUS, SENDING_STATUS, type AskSendStatus } from '../canvas/askSend';
import type { AskHold } from '../canvas/askHold';
import { tooSoon } from '../questionDock';
import { openFrameFile, openFrameLink } from '../store/browser';
import { logBridgeRefusal } from '../canvas/bridgeRefusals';
import { FrameBudgetContext } from '../canvas/frameBudgetScope';
import { budgetFor, type FrameSlot } from '../canvas/frameBudget';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { allows, blockedLine, escalate, grantedMode, setGrant, textKey, withInternet, withoutInternet, type BlockedWhat } from '../canvas/canvasGrants';
import type { BlockedWant } from '../canvasBridge';
import { noteCanvasUsed } from '../store/lastCanvas';
import { openCanvasExport } from '../store/canvasExport';
import { EXPORT_LABEL, type CanvasExportRef } from '../canvas/canvasExport';
import { ARTIFACT_STYLES } from '../canvas/artifactStyles';
export { ARTIFACT_STYLES } from '../canvas/artifactStyles';
import { CHANGES_NOT_KEPT, permissionTag, type CanvasHost } from '../canvas/hostContract';
import { Icon } from '../icons';
import type { TextBlock } from '../canvas/canvasText';
import '../styles/chat-canvas.css';

/**
 * Renders agent-authored HTML (a ```html or ```svg fenced block) as a live
 * artifact inside a HARD-SANDBOXED iframe.
 *
 * Security model — this is the load-bearing part:
 *   - sandbox="allow-scripts" WITHOUT allow-same-origin → the document runs
 *     in an opaque (null) origin. The host-owned runtime (by nonce) can measure
 *     height; artifact-authored scripts run only in a scripted policy.
 *   - A strict child CSP (injected as a <meta> and served as the artifact://
 *     header) caps what the artifact itself may load. The caller names the
 *     default (`defaultMode`); since 10-11 B1 there are no policy buttons: a
 *     refused load raises one permission line, and Allow is remembered per
 *     canvas (`canvas/canvasGrants.ts`).
 *   - The parent app CSP must allow `frame-src 'self'` for the srcdoc to load
 *     (see tauri.conf.json). Everything else stays locked down.
 *
 * The wrapper inherits a handful of the app's theme tokens as `--app-*` CSS
 * variables so artifacts visually match Hollow without being able to mutate it.
 */

const MAX_H = 640;
/** How long after the person pressed a canvas's Send its state post is still
 *  theirs (P1-33): the runtime's 120 ms debounce plus the message's trip. */
const STATE_GRACE_MS = 600;

/** Presentational base for chat artifacts — compact. The document paints the
 *  same --canvas the iframe element shows behind it, so its contrast holds on
 *  its own (axe inside a transparent frame assumes white; Canvas C2 K3). The
 *  default goes on body, never html: CSS propagates a body background to the
 *  viewport only while html has none, so an artifact's own
 *  `body{background:…}` still fills the frame (tests/smoke/artifact-background).
 *  The type scale (10-11 B2) is in `:where()`, so any rule an author writes
 *  wins, even a `*{margin:0}` reset, and sets no colour.
 *  Exported for the smoke harness's no-kit-script gallery frame. */

/** The document as nonce-slotted parts (`sandboxDocParts`). Height ping rides
 *  in via the host runtime. Artifact-authored scripts run only in a scripted
 *  policy (Canvas, Interactive). `ask`: the ask kit, which
 *  only an ask-canvas carries, and only with the kit it builds on. */
function buildDoc(code: string, mode: ArtifactMode, kit: CanvasKitAssets, ask: CanvasKitAssets, look?: Look | null): string[] {
  return sandboxDocParts(ARTIFACT_STYLES, mode, kit, `\n${code}\n`, kit.js ? ask : NO_KIT, look);
}

/** An ask-canvas's link to the conversation (Canvas C4 A2): the host side of
 *  bridge `draft` and `answered`. */
export interface AskBridge {
  /** The answer lines the person sent when this canvas revision is answered
   *  (locked), else null. Read reactively: a lock is posted to the frame. */
  answered: () => string | null;
  /** This view's own refusal (`not-current`, `unavailable`), or null. */
  refusal: () => DraftRefusal | null;
  /** Send the answers as the person's next message (question cards,
   *  2026-10-14). Called only after Hollow's own Send was clicked and the
   *  frame answered that click's `collect`. Resolves null once sent, else
   *  the refusal. */
  send: (d: { answers: DraftAnswers; summary: string }) => Promise<DraftRefusal | null>;
  /** The Send button's words (the form's `send`, else "Send answers"). */
  label: string;
  /** An earlier revision shown read-only: no Send at all. */
  past?: boolean;
  /** Answers Hollow holds for this revision (P1-33), the same in every copy:
   *  queued for after the running turn, or a Send that didn't go. */
  hold?: () => AskHold | null;
  /** Take them back: drop the hold; the answers stay in the canvas. */
  release?: () => void;
  /** The agent is answering: Send holds the answers for after the turn. */
  queues?: () => boolean;
  /** An earlier revision of this reply's canvas: no Send, Go to vN (false
   *  when vN's reply isn't in this transcript). */
  newer?: () => { revision: number; go: () => boolean } | null;
}

export interface HtmlArtifactProps {
  /** A host-owned look sample, without canvas controls or persistence claims. */
  headless?: boolean;
  /** Where a read-only frame's current editable version can be opened. */
  stateTarget?: { revision: number; open?: () => void };
  lookWorkspace?: string | null;
  /** Picker preview only; does not persist or alter the workspace. */
  look?: () => Look | null;
  /** Original source for host-converted documents; never grants execution. */
  sourceView?: { text: string; lang: string };
  /** Pinned live views receive only host-selected scalars and never inherit internet grants. */
  liveData?: () => import('../canvas/liveModel').LiveValues;
  localOnly?: boolean;
  /** The document body: the fence's HTML (a canvas passes its root's inner HTML). */
  code: string;
  lang?: string;
  /** The mode it opens in (Canvas C3: Canvas for canvases, and for html
   *  fences in quick and workspace chats). A grant the person gave this
   *  canvas (10-11 B1) raises it. */
  defaultMode?: ArtifactMode;
  /** Where the person's permission for this canvas is remembered (10-11
   *  B1). Default: a saved canvas's conversation and id, else a hash of
   *  `code`. */
  grantKey?: string;
  /** A canvas: its title in the head, its fence as the Code view and copy,
   *  and its version label or revision menu (`badge`, rendered as given:
   *  ChatCanvas's `CanvasVersions`). */
  canvas?: { title: string; source: string; badge?: JSX.Element; ref?: CanvasExportRef };
  /** An ask-canvas (`<hc-canvas ask>`, Canvas C4): the document also inlines
   *  the ask kit's input components. Read once, at mount. */
  ask?: boolean;
  /** Bridge init and state (Canvas C3). Every frame gets `init` on each load
   *  (kit version, locale, reduced motion); a saved canvas also gets its id
   *  and `state()`, and `onState` receives the state its frame posts (only
   *  its own canvas id, only while the frame has focus). No `onState`: the
   *  view is read-only and state changes stay in this mount. */
  bridge?: { canvasId: string; state: () => CanvasState; onState?: (state: CanvasState) => void; ask?: AskBridge; conversation?: string | null };
  /** The folder an `hc-file` link's relative path resolves against (bridge
   *  `navigate`): a curated view passes its workspace. Default: the
   *  conversation's folder (`chatCwd`), as for a reply's file links. */
  fileBase?: () => string | null;
  /** Opens the resolved file; default the side panel's Files view. */
  onFile?: (path: string) => void;
  /** A runtime error the frame reported over the bridge (`error` kind). */
  onError?: (message: string) => void;
  /** Repo ideas 2026-10-27 #1: once this load has settled and while this is
   *  set, ask the frame for its one design report (`design-check`) and pass
   *  the reply here. Untrusted, bounded text for an advisory row; nothing
   *  else reads it. */
  onDesign?: (report: DesignMessage) => void;
  /** Expand opens this elsewhere (the side panel) instead of growing in place. */
  onExpand?: () => void;
  /** The height to reserve before the frame reports its own (CSS px). */
  initialHeight?: number;
  /** Full height, no 640 px cap (the side panel). */
  fill?: boolean;
  /** Side-panel hosts only: permission and menu over the frame. */
  compact?: boolean;
  /** Plain version words for compact hosts, including conversation-only saves. */
  compactVersion?: string | null;
  compactControls?: { open: () => boolean; setOpen: (open: boolean) => void };
  /** Which host draws this canvas (plan L3 X7, `canvas/hostContract.ts`):
   *  `data-host` on the root, for the contract's smoke spec. */
  host?: CanvasHost;
  /** Its saved state in words, for the head (`savedWords`). */
  saved?: { text: string; detail: string } | null;
  /** An older version is on screen: its label (`olderLabel`, X8). */
  older?: string | null;
  /** The host's own ⋯ items, after the shared ones (Compare versions in
   *  the gallery). */
  actions?: ActionMenuItem[];
}

export function HtmlArtifact(props: HtmlArtifactProps) {
  // Plan L3 X9: 'text' is Read as text, the canvas's words in the page.
  const [view, setView] = createSignal<'preview' | 'code' | 'text'>('preview');
  const [controlsOpen, setControlsOpen] = createSignal(false);
  const showControls = () => props.compactControls?.open() ?? controlsOpen();
  const setShowControls = (open: boolean) => props.compactControls ? props.compactControls.setOpen(open) : setControlsOpen(open);
  let compactControls: HTMLDivElement | undefined;
  // 10-11 B1: the default policy, raised by a grant the person gave this
  // canvas for this exact content (`canvasGrants.ts`).
  const base = () => props.sourceView ? 'offline' : props.defaultMode ?? DEFAULT_ARTIFACT_MODE;
  const grantKey = () => props.grantKey ?? textKey(props.code);
  const [mode, setMode] = createSignal<ArtifactMode>(props.sourceView || props.localOnly ? base() : grantedMode(grantKey(), base(), props.code));
  // The permission line: the frame's first refusal this view can grant.
  const [asked, setAsked] = createSignal<{ want: BlockedWant; host: string } | null>(null);
  const [notNow, setNotNow] = createSignal(false);
  // Another document in this view (a new key, default or content) starts
  // from its own default and grant, never the previous one's.
  createEffect(on(() => [grantKey(), base(), props.code] as const, ([k, b, c]) => batch(() => {
    setMode(props.sourceView || props.localOnly ? b : grantedMode(k, b, c)); setAsked(null); setNotNow(false);
  }), { defer: true }));
  let pre: HTMLPreElement | undefined;
  let head: HTMLDivElement | undefined;
  const [expanded, setExpanded] = createSignal(false);
  const [reportedH, setReportedH] = createSignal(props.initialHeight ?? 140);
  let frame: HTMLIFrameElement | undefined;
  let frameScripted = false;
  // The state this frame last posted, so a reload (a mode switch) restores it
  // even before the store has it.
  let posted: { state: CanvasState } | null = null;
  const [changesNotKept, setChangesNotKept] = createSignal(false);
  createEffect(() => { if (props.bridge?.onState) setChangesNotKept(false); });
  createEffect(on(() => props.code, () => { posted = null; setChangesNotKept(false); }, { defer: true }));
  const currentCanvas = () => props.canvas && { ...props.canvas, ref: !props.localOnly && props.canvas.ref
    ? { ...props.canvas.ref, state: posted ? posted.state : props.bridge ? props.bridge.state() : props.canvas.ref.state } : undefined };

  // The Canvas Kit (inlined into every document) loads once, in its own
  // chunk. A signal, not a resource, so the first load never suspends a page.
  const [kit, setKit] = createSignal<CanvasKitAssets | null>(loadedCanvasKit());
  if (!kit()) void loadCanvasKit().then(setKit);
  // An ask-canvas also waits for the ask kit (its own chunk); others never load it.
  const [ask, setAsk] = createSignal<CanvasKitAssets | null>(props.ask ? loadedCanvasKitAsk() : NO_KIT);
  if (!ask()) void loadCanvasKitAsk().then(setAsk);
  const look = () => props.look ? props.look() : lookFor(props.lookWorkspace);
  const parts = createMemo(() => { const k = kit(), a = ask(); return k && a ? buildDoc(props.code, mode(), k, a, untrack(look)) : null; });
  // The transcript's frame budget (Canvas C3): at most six live frames. Off
  // screen this frame unmounts to a placeholder of its last height and
  // mounts again near the viewport; the head, the mode and the state the
  // frame last posted stay here, so the new load gets them through `init`.
  // Each mount stores the document again (`mountEpoch`): the artifact store
  // is a small FIFO, and a long transcript may have pushed this one out.
  // No scope (the side panel, the smoke harness): always live.
  const budget = useContext(FrameBudgetContext);
  const [live, setLive] = createSignal(!budget);
  const [mountEpoch, setMountEpoch] = createSignal(0);
  let slot: FrameSlot | undefined;
  const onLive = (on: boolean) => batch(() => {
    if (on === live()) return;
    if (on) setMountEpoch(n => n + 1);
    else { stopTheme?.(); stopTheme = undefined; frame = undefined; }
    setLive(on);
  });
  const watchSlot = (el: HTMLDivElement) => {
    if (!budget) return;
    slot?.dispose();
    // A curated section view in the side panel shares the workspace
    // transcript's budget as a pinned slot (Canvas C6 · K2).
    slot = budget.of ? budgetFor(budget.of).register(el, onLive, { pinned: true }) : budgetFor(budget).register(el, onLive);
  };
  onCleanup(() => slot?.dispose());

  // Outside Tauri (no artifact:// URL) the document falls back to srcdoc, which
  // inherits the app CSP: Canvas renders Offline there rather than pretending.
  // The fallback seals the parts with a nonce of its own.
  const fallbackSrcdoc = createMemo(() => {
    const k = kit(), p = parts();
    if (!k || !p) return '';
    return sealDoc(srcdocMode(mode()) === mode() ? p : buildDoc(props.code, srcdocMode(mode()), k, ask() ?? NO_KIT, untrack(look)));
  });
  // Read the URL only once it is stored: reading a resource while it loads
  // (a remount's re-store) would suspend the page around this transcript.
  const stored = () => !docUrl.loading && docUrl.state === 'ready';
  const fallbackNote = () => stored() && !docUrl() && srcdocMode(mode()) !== mode();
  const what = (): BlockedWhat => (props.canvas ? 'canvas' : 'html');
  function onBlocked(want: BlockedWant, host: string) {
    // Outside the app a Canvas document renders Offline: its refused scripts
    // are the fallback's, not something to grant.
    if (asked() || notNow() || allows(mode(), want)) return;
    setAsked({ want, host });
  }
  function grant(next: ArtifactMode | null) {
    if (props.sourceView || props.localOnly) return;
    setGrant(grantKey(), next, props.code);
    batch(() => { setAsked(null); setMode(next ?? base()); });
    // The clicked button is gone: keep focus on this canvas's head.
    queueMicrotask(() => head?.querySelector<HTMLElement>('summary')?.focus());
  }
  const allow = () => { const a = asked(); if (a) grant(escalate(mode(), a.want)); };
  function toggleSource() {
    if (props.headless) return;
    setView(v => (v === 'code' ? 'preview' : 'code'));
    queueMicrotask(() => (view() === 'code' ? pre : head?.querySelector<HTMLElement>('summary'))?.focus());
  }
  // Read as text (plan L3 X9, 2026-10-14; review U canvases): the stored
  // HTML's words, headings, lists and tables, drawn by the host as text
  // only. The parser is its own chunk, loaded the first time it is asked for.
  let textPanel: HTMLDivElement | undefined;
  const [textBlocks, setTextBlocks] = createSignal<TextBlock[] | null>(null);
  createEffect(on(() => (view() === 'text' ? props.canvas?.source ?? props.code : null), (src) => {
    if (src === null) return;
    setTextBlocks(null);
    void import('../canvas/canvasText').then(
      (m) => { if (view() === 'text') setTextBlocks(m.readAsText(src)); },
      () => setTextBlocks([]),
    );
  }));
  function toggleText() {
    setView(v => (v === 'text' ? 'preview' : 'text'));
    queueMicrotask(() => (view() === 'text' ? textPanel : head?.querySelector<HTMLElement>('summary'))?.focus());
  }
  // The permission boundary, said before anything is refused (plan L3 X7):
  // the policy this frame renders in now (outside the app, the fallback's).
  // Read in the head only (it needs `docUrl` and `label`, defined below).
  const perm = () => permissionTag(fallbackNote() ? srcdocMode(mode()) : mode(), base(), !!props.localOnly, label());
  const hasNetwork = () => mode() === 'images' || mode() === 'https' || mode() === 'interactive';
  const menu = (): ActionMenuItem[] => [
    ...(props.compact ? [{ label: compactLabel(), informational: true, action: () => {} }] : []),
    ...(props.compact && props.host === 'expanded' ? [{
      label: 'Versions and pin…', action: () => {
        setShowControls(true);
        queueMicrotask(() => (compactControls?.querySelector<HTMLElement>('button, summary') ?? compactControls)?.focus());
      },
    }] : []),
    { label: view() === 'code' ? `Show ${label()}` : 'View source', action: toggleSource, shortcut: { label: '⌥⌘U', aria: 'Alt+Meta+U' } },
    { label: 'Copy source', action: copy },
    { label: view() === 'text' ? `Show ${label()}` : 'Read as text', action: toggleText },
    ...(props.canvas && !props.localOnly ? [{ label: props.canvas.ref?.workspaceId ? EXPORT_LABEL : `${EXPORT_LABEL} (save in a workspace first)`, disabled: !props.canvas.ref?.workspaceId, action: () => openCanvasExport(currentCanvas()!) }] : []),
    ...(props.actions ?? []),
    ...(!props.sourceView && !props.localOnly && mode() === 'offline' ? [{ label: 'Run scripts in this HTML', action: () => grant(escalate(mode(), 'scripts')), separatorBefore: true }] : []),
    ...(props.sourceView || props.localOnly ? [] : hasNetwork()
      ? [{ label: `Turn internet off for this ${label()}`, action: () => { const off = withoutInternet(mode()); grant(off === base() ? null : off); }, separatorBefore: mode() !== 'offline' }]
      : [{ label: `Allow internet for this ${label()}`, action: () => grant(withInternet(mode())), separatorBefore: mode() !== 'offline' }]),
    ...(!props.sourceView && mode() === 'canvas' && base() === 'offline' ? [{ label: 'Turn scripts off again', action: () => grant(null) }] : []),
  ];
  // artifact:// load path — the served response carries its own CSP, so the
  // selected policy is authoritative. Null outside Tauri → srcdoc fallback.
  const [docUrl] = createResource(
    () => { const p = parts(); return p && live() ? { parts: p, mode: mode(), mount: mountEpoch() } : false; },
    ({ parts, mode }) => artifactDocUrl(parts, mode),
  );

  // Bridge `drafted`/`answered` (C4): posted to whichever frame is live now.
  const post = (m: unknown) => { try { frame?.contentWindow?.postMessage(m, '*'); } catch { /* frame gone */ } };
  createEffect(() => { const values = props.liveData?.(); if (values) post({ __hc: 1, kind: 'live-data', values }); });
  // Ask-canvas Send (question cards, 2026-10-14): Hollow's own button,
  // under the frame. Its click posts `collect` with a fresh nonce; only the
  // frame's reply carrying that nonce, within COLLECT_TIMEOUT_MS, is sent.
  // A draft the frame posts on its own is refused (`unrequested`), so no
  // canvas script can send for the person.
  let collecting: { nonce: string; at: number; timer: ReturnType<typeof setTimeout> } | null = null;
  const [sending, setSending] = createSignal(false);
  // A collect is out (the frame hasn't handed its answers over yet).
  const [asking, setAsking] = createSignal(false);
  // Send is off from the click until the send settles (2026-10-15): a
  // second click while the frame hands over its answers, or while they are
  // queued, does nothing. Before, only the queueing half was covered, and a
  // click while collecting started a fresh collect.
  const inFlight = () => asking() || sending();
  // And while another copy's send is on its way or the answers wait for
  // after the running turn (P1-33; the same hold in every copy of this
  // revision): a second Send would send them twice.
  const held = () => { const k = props.bridge?.ask?.hold?.()?.kind; return k === 'queued' || k === 'sending'; };
  // A Send that didn't go (P1-33): the group becomes a callout with Send now
  // and Take them back, in every copy; the answers never go to the box.
  const unsent = () => props.bridge?.ask?.hold?.()?.kind === 'not-sent';
  const sendLabel = () => (props.bridge?.ask?.queues?.() ? QUEUE_LABEL : props.bridge?.ask?.label);
  const [askStatus, setAskStatus] = createSignal<AskSendStatus | null>(null);
  // The status line's id, so Send can point at the reason it is off.
  const askStatusId = createUniqueId();
  // A press that is refused again sets the same line, which a live region
  // doesn't read twice: clear it first (the line stays mounted, so the
  // region is still there to read it).
  const sayAskStatus = (next: AskSendStatus) => {
    if (askStatus()?.text !== next.text) { setAskStatus(next); return; }
    setAskStatus({ ...next, text: '' });
    queueMicrotask(() => setAskStatus(next));
  };
  // The line follows the hold: queued while it waits, the callout's words
  // while a Send didn't go, and nothing once either ends (sent, taken back).
  createEffect(on(() => props.bridge?.ask?.hold?.()?.kind ?? null, (now, before) => {
    if (now === 'sending') setAskStatus(SENDING_STATUS);
    else if (now === 'queued') setAskStatus(QUEUED_STATUS);
    else if (now === 'not-sent') setAskStatus(NOT_SENT_STATUS);
    else if (before) setAskStatus(null);
  }));
  // Take them back: Send is back, and focus goes to it (the callout's
  // buttons it was on are gone).
  let sendGroup: HTMLDivElement | undefined;
  function takeBack() {
    props.bridge?.ask?.release?.();
    queueMicrotask(() => sendGroup?.querySelector<HTMLElement>('.chat-ask-send-btn')?.focus());
  }
  const endCollect = () => { if (collecting) clearTimeout(collecting.timer); collecting = null; setAsking(false); };
  onCleanup(endCollect);
  // A refused message from this frame is logged (C6 P1): kind, size,
  // reason and ids, never what it carried.
  const refused = (data: unknown, reason: Parameters<typeof logBridgeRefusal>[1]) =>
    logBridgeRefusal(data, reason, props.bridge?.canvasId ?? null, props.bridge?.conversation ?? null);
  // When the frame's height last changed (ms), which moves Send under it: a
  // pointer click in the half second after is refused (2026-10-15), as the
  // dock refuses keys, since the button may have slid under a pointer aimed
  // at something else. A key press (`detail` 0) on the focused button stays.
  let movedAt: number | null = null;
  // When the person last pressed this canvas's Send (or Send now): the
  // answer they chose just before may still be on its way as state.
  let pressedAt: number | null = null;
  // `again`: the callout's Send now, which collects the form as it is now
  // (the person may have changed an answer since the send that didn't go).
  function pressSend(e: MouseEvent, again = false) {
    const a = props.bridge?.ask;
    if (!a || !e.isTrusted || inFlight()) return;
    pressedAt = Date.now();
    const kind = a.hold?.()?.kind;
    if (held() || (unsent() && !again)) { sayAskStatus(kind === 'sending' ? SENDING_STATUS : kind === 'queued' ? QUEUED_STATUS : NOT_SENT_STATUS); return; }
    const why = a.answered() !== null ? 'locked' : a.refusal();
    if (why) { sayAskStatus(askRefusalStatus(why)); return; }
    // A Try again refused as moved keeps its Try again.
    if (e.detail > 0 && tooSoon(movedAt, Date.now())) { sayAskStatus({ ...MOVED_STATUS, retry: askStatus()?.retry }); return; }
    endCollect();
    const nonce = newCollectNonce();
    collecting = {
      nonce, at: Date.now(),
      timer: setTimeout(() => {
        if (collecting?.nonce !== nonce) return;
        collecting = null;
        setAsking(false);
        setAskStatus({ text: 'The canvas didn’t hand over its answers.', tone: 'warning', retry: true });
      }, COLLECT_TIMEOUT_MS),
    };
    setAsking(true);
    setAskStatus(SENDING_STATUS);
    post(collectMessage(nonce));
  }
  function onDraft(m: DraftMessage, data: unknown) {
    const a = props.bridge?.ask;
    const now = Date.now();
    const requested = !!collecting && collecting.nonce === m.nonce && now - collecting.at <= COLLECT_TIMEOUT_MS;
    if (requested) endCollect();
    const why = draftRefusal({
      ask: !!props.ask, locked: (a?.answered() ?? null) !== null, host: a ? a.refusal() : 'unavailable',
      requested, pending: sending(), now, last: null,
    }) ?? (a ? null : 'unavailable');
    if (why) {
      refused(data, why);
      if (requested) setAskStatus(askRefusalStatus(why));
      return;
    }
    setSending(true);
    a!.send({ answers: m.answers, summary: m.summary })
      .catch((): DraftRefusal => 'send-failed')
      .then((r) => {
        setSending(false);
        if (r) { refused(data, r); setAskStatus(askRefusalStatus(r)); }
        else setAskStatus(held() ? QUEUED_STATUS : { text: 'Sent.', tone: 'success' });
      });
  }
  const showSend = () => !!props.ask && !!props.bridge?.ask && !props.bridge.ask.past && props.bridge.ask.answered() === null && !props.bridge.ask.newer?.();
  const [newerNote, setNewerNote] = createSignal('');
  const newer = () => (props.ask &&props.bridge?.ask?.answered() === null ? props.bridge.ask.newer?.() ?? null : null);
  // A lock reaches a live frame at once (and every later load, after init).
  createEffect(() => { if (props.bridge?.ask?.answered() != null) post(answeredMessage(props.bridge.ask.answered()!)); });

  function onMsg(e: MessageEvent) {
    const r = acceptFrameMessage(e, frame, props.bridge?.canvasId ?? null);
    if (!r.ok && r.reason !== 'source' && r.reason !== 'origin') {
      refused(e.data, r.reason);
      // A reply to Hollow's Send that isn't valid for this canvas: say so
      // under the frame. Never sends.
      const kind = (e.data as { kind?: unknown } | null)?.kind;
      if (collecting && (kind === 'draft' || kind === 'ask-invalid')) {
        endCollect();
        setAskStatus(askRefusalStatus('invalid'));
      }
    }
    if (!r.ok || !frame) return;
    const m = r.message;
    if (m.kind === 'size') {
      const was = frameHeight();
      setReportedH(Math.max(40, Math.min(m.h, 4000)));
      if (frameHeight() !== was) movedAt = Date.now();
    }
    else if (m.kind === 'open') openFrameLink(m, frame, frameScripted);
    else if (m.kind === 'navigate') openFrameFile(m.target, frame, props.fileBase?.(), props.onFile);
    else if (m.kind === 'blocked') onBlocked(m.want, m.host);
    else if (m.kind === 'source') toggleSource();
    else if (m.kind === 'error') props.onError?.(m.message);
    else if (m.kind === 'state') {
      // Only while the person is using this frame: a canvas script that
      // posts state as it loads is not the person leaving a view, and
      // viewing never writes. P1-33: or just after the person pressed this
      // canvas's Send (the runtime posts on a 120 ms debounce, so an answer
      // chosen right before Send arrives once focus is on the button, and a
      // failed send may redraw the frame from the saved state).
      if (document.activeElement !== frame && !(pressedAt !== null && Date.now() - pressedAt < STATE_GRACE_MS)) return refused(e.data, 'focus');
      posted = { state: m.state };
      if (props.canvas) noteCanvasUsed(currentCanvas()!);
      if (props.bridge?.onState) props.bridge.onState(m.state);
      else if (!props.headless) setChangesNotKept(true);
    }
    else if (m.kind === 'draft') onDraft(m, e.data);
    else if (m.kind === 'ask-invalid') {
      if (collecting?.nonce !== m.nonce) return refused(e.data, 'unrequested');
      endCollect();
      setAskStatus({ text: m.count === 1 ? 'One answer needs attention.' : `${m.count} answers need attention.`, tone: 'danger' });
    }
    else if (m.kind === 'settled') setFrameSettled(true);
    else if (m.kind === 'design') {
      // Only the reply to this load's own request, once.
      if (!designTicket || m.ticket !== designTicket) return refused(e.data, 'unrequested');
      designTicket = null;
      props.onDesign?.(m);
    }
  }
  // Ideas 2026-10-27 #1: one design check per load, asked once the frame
  // has settled and the host wants it (a reply canvas whose reply ended).
  const [frameSettled, setFrameSettled] = createSignal(false);
  let designTicket: string | null = null, designAsked = false;
  createEffect(() => {
    if (!frameSettled() || !props.onDesign || designAsked) return;
    designAsked = true;
    designTicket = newCollectNonce();
    post(designCheckMessage(designTicket));
  });
  function postInit(el: HTMLIFrameElement) {
    // A new load settles (and may be checked) afresh.
    setFrameSettled(false); designAsked = false; designTicket = null;
    const b = props.bridge;
    const reducedMotion = motionReduced();
    const init = initMessage({ canvasId: b?.canvasId ?? null, kit: kit()?.version ?? '', locale: navigator.language || 'en', reducedMotion, state: posted ? posted.state : b?.state() ?? null });
    try { el.contentWindow?.postMessage(init, '*'); } catch { /* frame gone */ }
    if (props.liveData) { try { el.contentWindow?.postMessage({ __hc: 1, kind: 'live-data', values: props.liveData() }, '*'); } catch { /* frame gone */ } }
    const answered = b?.ask?.answered() ?? null;
    if (answered !== null) { try { el.contentWindow?.postMessage(answeredMessage(answered), '*'); } catch { /* frame gone */ } }
  }
  let stopTheme: (() => void) | undefined;
  onMount(() => window.addEventListener('message', onMsg));
  onCleanup(() => { window.removeEventListener('message', onMsg); stopTheme?.(); });

  async function copy() {
    try {
      await navigator.clipboard.writeText(props.sourceView?.text ?? props.canvas?.source ?? props.code);
      flashFooter('Copied', 1000);
    } catch {
      flashFooter('× copy failed', 1500);
    }
  }

  const frameHeight = () => (props.fill ? reportedH() : expanded() ? Math.max(reportedH(), MAX_H) : Math.min(reportedH(), MAX_H));
  const label = () => (props.canvas ? 'canvas' : props.lang === 'mermaid' ? 'diagram' : props.lang === 'svg' ? 'graphic' : 'HTML');
  const compactLabel = () => [
    props.canvas?.title ?? `Rendered ${label()}`,
    props.older ?? props.compactVersion,
    props.saved?.text,
    changesNotKept() ? 'changes not kept' : null,
  ].filter(Boolean).join(' · ');

  return (
    <div class="chat-artifact" classList={{ expanded: expanded(), canvas: !!props.canvas, 'artifact-compact': !!props.compact }} data-sandbox={mode()} data-view={view()} data-host={props.host}
      onKeyDown={(e) => { if (e.metaKey && e.altKey && !e.ctrlKey && !e.shiftKey && e.code === 'KeyU') { e.preventDefault(); toggleSource(); } }}
      onPointerDown={() => { if (props.canvas) noteCanvasUsed(currentCanvas()!); }}
      onFocusIn={() => { if (props.canvas) noteCanvasUsed(currentCanvas()!); }}>
      <Show when={!props.headless}><div class="chat-artifact-head" ref={head}>
        <Show when={!props.compact}>
        <span class="chat-artifact-badge">{props.canvas ? 'CANVAS' : (props.lang || 'html').toUpperCase()}</span>
        <Show when={props.canvas} fallback={<span class="chat-artifact-title">rendered {label()}</span>}>
          {(c) => <>
            <span class="chat-artifact-title canvas-title" title={c().title}>{c().title}</span>
            {c().badge}
          </>}
        </Show>
        {/* Plan L3 X7/X8: which version is on screen and whether it is kept, in words. */}
        <Show when={props.older}>{(o) => <span class="chat-artifact-state chat-artifact-older" data-older="">{o()}</span>}</Show>
        <Show when={props.saved}>{(s) => <span class="chat-artifact-state" data-saved-state="" title={s().detail}>{s().text}</span>}</Show>
        <Show when={changesNotKept()}><span class="chat-artifact-state">· changes not kept</span></Show>
        <span class="chat-artifact-spacer" />
        </Show>
        {/* The permission boundary, before anything is refused (plan L3 X7);
            while a grant lets it reach the internet, it says so (10-11 B1). */}
        <span class="chat-artifact-perm" classList={{ 'chat-artifact-net': perm().kind === 'internet' }} data-permission={perm().kind} title={perm().detail} role={props.compact ? 'img' : undefined} aria-label={props.compact ? perm().text : undefined}>
          <Show when={props.compact} fallback={perm().text}><Icon name={perm().kind === 'internet' ? 'globe' : 'lock'} size={14}/></Show>
        </span>
        <Show when={!props.compact}>
        <Show when={props.onExpand} fallback={
          <Show when={!props.fill}>
            <button class="chat-artifact-btn" type="button" title={expanded() ? 'Collapse' : 'Expand'} aria-label={expanded() ? 'Collapse artifact' : 'Expand artifact'} onClick={() => setExpanded(v => !v)}>{expanded() ? '▢' : '⤢'}</button>
          </Show>
        }>
          <button class="chat-artifact-btn" type="button" title="Open full height in the Preview pane" onClick={() => props.onExpand?.()}>Expand</button>
        </Show>
        </Show>
        <ActionMenu label={props.canvas ? 'Canvas actions' : 'HTML actions'} items={menu()} />
      </div></Show>
      <Show when={changesNotKept()}><div class="artifact-ask" role="status">
        <span>{CHANGES_NOT_KEPT}{' '}
          <Show when={props.stateTarget?.open} fallback="The editable source is unavailable here.">
            <button type="button" class="chat-canvas-back" onClick={() => props.stateTarget?.open?.()}>Open v{props.stateTarget?.revision}</button>{' '}to keep them
          </Show>
        </span>
      </div></Show>
      <Show when={props.compact && showControls()}>
        <div class="artifact-compact-controls" ref={compactControls} role="group" aria-label="Canvas versions and pin" tabIndex={-1}
          onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setShowControls(false); head?.querySelector<HTMLElement>('summary')?.focus(); } }}>
          {props.canvas?.badge}
          <button type="button" class="chat-artifact-btn" onClick={() => { setShowControls(false); head?.querySelector<HTMLElement>('summary')?.focus(); }}>Close controls</button>
        </div>
      </Show>
      {/* 10-11 B1: one permission line, only after the frame was refused
          something this view can grant (no policy buttons). */}
      <Show when={view() === 'preview' && ((!props.sourceView && !props.localOnly && asked()) || fallbackNote())}>
        <div class="artifact-ask" role="status" data-want={asked()?.want ?? 'fallback'}>
          <Show when={asked()} fallback={<span class="artifact-ask-text">Scripts run only in the Hollow app; showing this without them.</span>}>
            {(a) => <>
              <span class="artifact-ask-text">{blockedLine(a().want, a().host, what())}</span>
              <button type="button" class="btn small" onClick={allow}>{a().want === 'scripts' ? 'Run scripts' : 'Allow'}</button>
              <button type="button" class="btn small ghost" onClick={() => batch(() => { setAsked(null); setNotNow(true); })}>Not now</button>
            </>}
          </Show>
        </div>
      </Show>
      <Switch>
        <Match when={view() === 'code'}>
          <pre class="chat-artifact-code" ref={pre} tabindex="0" aria-label={`Source of this ${label()}`}><code>{props.sourceView?.text ?? props.canvas?.source ?? props.code}</code></pre>
        </Match>
        <Match when={view() === 'text'}>
          {/* Text only, never markup: nothing from the canvas runs here. */}
          <div class="chat-artifact-text" ref={textPanel} tabindex="0" role="region" aria-label={`Text of ${props.canvas?.title ?? `this ${label()}`}`}>
            <Show when={textBlocks()} fallback={<p class="chat-artifact-text-note">Reading the text…</p>}>
              {(blocks) => <Show when={blocks().length} fallback={<p class="chat-artifact-text-note">This {label()} has no text to read. View source shows everything in it.</p>}>
                <For each={blocks()}>{(b) => <CanvasTextBlock block={b} />}</For>
              </Show>}
            </Show>
            <button type="button" class="chat-artifact-btn chat-artifact-text-back" onClick={toggleText}>Show {label()}</button>
          </div>
        </Match>
        <Match when={view() === 'preview'}>
        <div class="chat-artifact-slot" ref={watchSlot} data-frame-live={live() ? 'true' : 'false'}>
          <Show when={live() && parts() && stored()} fallback={
            <div class="chat-artifact-placeholder" data-frame-placeholder="" style={{ height: `${frameHeight()}px` }}>
              {/* Both preparation promises resolve on failure: the kit uses
                  light DOM and artifactDocUrl returns the offline srcdoc fallback. */}
              <Show when={live()}><CanvasPreparation title={props.canvas?.title ?? label()} height={frameHeight()} loading /></Show>
              <Show when={budget && !live()}>
                <span class="chat-artifact-placeholder-label">Paused while out of view</span>
                <button type="button" class="chat-artifact-btn" onClick={() => slot?.touch()}>Show {label()}</button>
              </Show>
            </div>
          }>
            <iframe
              ref={el => { frame = el; frameScripted = modeRunsScripts(mode()); stopTheme?.(); stopTheme = watchFrameTheme(el, look); el.addEventListener('load', () => postInit(el)); el.addEventListener('focus', () => slot?.touch()); }}
              class="chat-artifact-frame"
              sandbox={SANDBOX}
              src={docUrl() ?? undefined}
              srcdoc={docUrl() ? undefined : fallbackSrcdoc()}
              style={{ height: `${frameHeight()}px` }}
              title={props.canvas ? `Canvas: ${props.canvas.title}` : `Rendered ${label()} artifact`}
              aria-label={props.compact ? compactLabel() : undefined}
              loading="lazy"
            />
          </Show>
        </div>
        <Show when={showSend()}>
          <div class="chat-ask-send" ref={sendGroup} role="group" aria-label="Send this canvas’s answers" data-unsent={unsent() ? '' : undefined}>
            <Show when={!unsent()}>
              <button type="button" class="btn primary sm chat-ask-send-btn" aria-disabled={inFlight() || held() ? 'true' : undefined} aria-describedby={askStatus() ? askStatusId : undefined} data-held={held() && !inFlight() ? '' : undefined} onClick={pressSend}>{sendLabel()}</button>
              <span class="chat-ask-send-note">{props.bridge?.ask?.queues?.() ? 'Holds your answers until the agent finishes, then sends them as your next message.' : 'Sends your answers to the agent as your next message.'}</span>
            </Show>
            {/* Always mounted, empty until there is something to say: a live
                region inserted together with its first message often isn't
                read (lane H review of #631). */}
            <p class="chat-ask-send-status" id={askStatusId} role="status" data-tone={askStatus()?.tone} data-empty={askStatus()?.text ? undefined : ''}>{askStatus()?.text ?? ''}</p>
            <Show when={unsent()}>
              <button type="button" class="btn primary sm" aria-disabled={inFlight() ? 'true' : undefined} onClick={(e) => pressSend(e, true)}>Send now</button>
              <button type="button" class="btn ghost sm" onClick={takeBack}>Take them back</button>
            </Show>
            <Show when={!unsent() && askStatus()?.retry}><button type="button" class="btn ghost sm" onClick={pressSend}>Try again</button></Show>
          </div>
        </Show>
        <Show when={newer()}>
          {(n) => (
            <div class="chat-ask-send" role="group" aria-label="This canvas’s answers">
              <span class="chat-ask-send-note">Answers go from the current version.</span>
              <button type="button" class="btn sm" onClick={() => setNewerNote(n().go() ? '' : `v${n().revision}’s reply isn’t in this conversation’s view. Scroll to it to answer there, or reply in the message box.`)}>Go to v{n().revision}</button>
              <p class="chat-ask-send-status" role="status" data-tone="warning" data-empty={newerNote() ? undefined : ''}>{newerNote()}</p>
            </div>
          )}
        </Show>
        </Match>
      </Switch>
    </div>
  );
}

/** One block of Read as text: a heading, a paragraph, a list, a data table
 *  (a chart's data) or preformatted text. */
function CanvasTextBlock(props: { block: TextBlock }) {
  const b = props.block;
  switch (b.kind) {
    case 'heading': return <p class="chat-artifact-text-heading" role="heading" aria-level={b.level === 2 ? 4 : 5}>{b.text}</p>;
    case 'text': return <p>{b.text}</p>;
    case 'list': return b.ordered
      ? <ol><For each={b.items}>{(i) => <li>{i}</li>}</For></ol>
      : <ul><For each={b.items}>{(i) => <li>{i}</li>}</For></ul>;
    case 'pre': return <>{b.label ? <p class="chat-artifact-text-heading">{b.label}</p> : null}<pre>{b.text}</pre></>;
    case 'table': return (
      <table>
        {b.caption ? <caption>{b.caption}</caption> : null}
        {b.head ? <thead><tr><For each={b.head}>{(h) => <th scope="col">{h}</th>}</For></tr></thead> : null}
        <tbody><For each={b.rows}>{(r) => <tr><For each={r}>{(c) => <td>{c}</td>}</For></tr>}</For></tbody>
      </table>
    );
  }
}
