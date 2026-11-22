// The Preview pane (run 15 P1): the app's one right-hand pane. It shows only
// what the person opened (a file, a web page, a conversation's canvas) and
// is hidden otherwise: nothing opens it by itself when a turn ends. Pure
// state, importing nothing from the rest of the store, so every caller
// (chat chips, Results and Library rows, links, the agent's navigate) can
// use `openPreview` without an import cycle. PreviewPane.tsx renders it.
import { batch, createSignal } from 'solid-js';

export type PreviewTarget =
  /** An absolute path, or an attached image's data URL; `line`/`end` cite lines. */
  | { kind: 'file'; path: string; line?: number; end?: number }
  /** A web page (http or https), shown in the pane's web mode. An empty
   *  address is the web mode's start page (a saved page review). */
  | { kind: 'url'; url: string }
  /** A canvas or HTML visual of the open conversation (a canvas's Expand). */
  | { kind: 'visual'; key: string }
  /** A chat's subagents (COO rebuild L3): one tab per chat, keyed by its
   *  conversation; `open` is the subagent whose session the tab shows. */
  | { kind: 'subagents'; parent: string; open?: string }
  /** A build workspace's Changes: its changed files, diff, commit and
   *  publish (COO rebuild L4: it was the workspace panel's Changes section). */
  | { kind: 'changes'; workspaceId: string }
  /** A project script's output (4.5, A10): one tab per script, showing its run's log, Run and Stop. */
  | { kind: 'script'; workspaceId: string; scriptId: string }
  /** Any git folder's Diff tab (parity 5.3): the chat folder's changes with a
   *  file tree, Find and Viewed, or one turn's changes. One tab per folder. */
  | { kind: 'diff'; cwd: string }
  /** The pull request of a git folder's branch, with checks and comments (parity 5.4). */
  | { kind: 'pr'; cwd: string }
  /** A chat's terminals (parity 7.2): one tab per chat holding its shells, which
   *  live in `store/terminals.ts`, so leaving the tab keeps them running. */
  | { kind: 'terminal'; chat: string }
  /** A New tab (overnight 11-12 C5): the pane's Tools (Files, Browser,
   *  Subagents, Changes); picking one opens it in this tab. It belongs to
   *  the chat it was opened on (`chat`; `DRAFT_TAB_OWNER` for a chat with
   *  no key yet), like a Subagents tab, and goes when another comes on screen. */
  | { kind: 'newtab'; id: number; chat: string };

/** How many opened items back and forward walk. */
export const PREVIEW_HISTORY_CAP = 30;
/** One app-wide document list, like the web tabs; history remains independent. */
export const PREVIEW_DOCUMENT_CAP = 12;
type DocumentTarget = Exclude<PreviewTarget, { kind: 'url' }>;
export const [previewDocuments, setPreviewDocuments] = createSignal<DocumentTarget[]>([]);
export const previewDocumentKey = (target: DocumentTarget) => JSON.stringify([target.kind, target.kind === 'file' ? target.path : target.kind === 'subagents' ? target.parent : target.kind === 'visual' ? target.key : target.kind === 'newtab' ? target.id : target.kind === 'diff' || target.kind === 'pr' ? target.cwd : target.kind === 'terminal' ? target.chat : target.kind === 'script' ? `${target.workspaceId}:${target.scriptId}` : target.workspaceId]);
function rememberDocument(target: PreviewTarget | undefined) {
  if (!target || target.kind === 'url') return;
  const key = previewDocumentKey(target);
  setPreviewDocuments(list => {
    const at = list.findIndex(t => previewDocumentKey(t) === key);
    return at < 0 ? [...list, target].slice(-PREVIEW_DOCUMENT_CAP) : list.map((t, i) => i === at ? target : t);
  });
}

const [items, setItems] = createSignal<PreviewTarget[]>([]);
const [at, setAt] = createSignal(-1);
const [shown, setShown] = createSignal(false);
/** Chats show their summary whenever the pane is closed. No dismissal state. */
export const previewCardShown = () => !previewShown();
/** Bumped to make the pane re-read its file (opened again, Refresh, an agent edit). */
export const [previewNonce, setPreviewNonce] = createSignal(0);
/** An address the web mode should load: set by an open, back or forward to a
 *  web page (a fresh object each time, so a second click on the same link
 *  asks again), never by ⌘\ reopening the pane, which shows its tab as it
 *  was left. The pane takes it once (`takePreviewWebLoad`). */
const [webLoad, setWebLoad] = createSignal<{ url: string } | null>(null);
export const previewWebLoad = webLoad;
export function takePreviewWebLoad(): string | null {
  const load = webLoad();
  if (load) setWebLoad(null);
  return load?.url ?? null;
}
const askWebLoad = (target: PreviewTarget | undefined) => setWebLoad(target?.kind === 'url' && target.url ? { url: target.url } : null);

/** What the pane shows now; null while it is hidden. */
export const previewTarget = (): PreviewTarget | null => (shown() ? items()[at()] ?? null : null);
export const previewShown = () => previewTarget() !== null;
/** Whether ⌘\ has something to bring back (an item opened this session). */
export const previewRemembered = () => !!items()[at()];
export const canPreviewBack = () => shown() && at() > 0;
export const canPreviewForward = () => shown() && at() < items().length - 1;

/** A string as a target: an http(s) address, an image data URL or an
 *  absolute path. Null for anything else (a relative path, another scheme). */
export function targetOf(text: string): PreviewTarget | null {
  const value = text.trim();
  if (!value) return null;
  if (/^https?:\/\//i.test(value)) return { kind: 'url', url: value };
  if (/^data:image\//i.test(value)) return { kind: 'file', path: value };
  if (value.startsWith('/')) return { kind: 'file', path: value };
  return null;
}

function clean(target: PreviewTarget): PreviewTarget {
  if (target.kind === 'subagents') return target.open ? { kind: 'subagents', parent: target.parent, open: target.open } : { kind: 'subagents', parent: target.parent };
  if (target.kind !== 'file') return target;
  const { path, line, end } = target;
  if (!Number.isSafeInteger(line) || line! < 1) return { kind: 'file', path };
  return Number.isSafeInteger(end) && end! >= line! ? { kind: 'file', path, line, end } : { kind: 'file', path, line };
}

const same = (a: PreviewTarget | undefined, b: PreviewTarget) => !!a && JSON.stringify(a) === JSON.stringify(b);

/** The newest thing asked of the pane (2026-11-04 review): a file click
 *  takes a number in its handler (`fileClick`), and its lookup opens only if
 *  no other open, close, ⌘\\, Back or Forward came after it. */
let previewIntent = 0;
export function newPreviewIntent(): number { return ++previewIntent; }
export const isLatestPreviewIntent = (n: number) => n === previewIntent;
/** The current number, taken without making a new intent: a tool's lookup
 *  stamps it before its await (see `newTabLookupStillValid`). */
export const previewIntentNow = () => previewIntent;

/** Show `target` in the Preview pane (a person's click, or the agent's
 *  navigate). Opening what is already shown re-reads it. */
export function openPreview(target: PreviewTarget | string): void {
  const next = typeof target === 'string' ? targetOf(target) : clean(target);
  if (!next) return;
  previewIntent++;
  batch(() => {
    rememberDocument(next);
    askWebLoad(next);
    const list = items();
    if (shown() && same(list[at()], next)) { setPreviewNonce(n => n + 1); return; }
    const kept = [...list.slice(0, at() + 1), next].slice(-PREVIEW_HISTORY_CAP);
    setItems(kept);
    setAt(kept.length - 1);
    setShown(true);
    setPreviewNonce(n => n + 1);
  });
}

/** Hide the pane. Its history stays, for ⌘\ and back/forward. */
export function closePreview(): void { previewIntent++; setShown(false); }

/** Selecting a live web tab preserves its current navigation, including forms. */
export function selectPreviewWeb(url: string): void {
  batch(() => { openPreview({ kind: 'url', url }); setWebLoad(null); });
}

/** Pane tabs closed by hand, newest last, so Reopen closed view can bring one
 *  back (t3code parity 4.6, A14). A New tab is never kept; a tab dropped by
 *  moving to another conversation is not a close. */
export const CLOSED_VIEW_CAP = 10;
const [closedViews, setClosedViews] = createSignal<DocumentTarget[]>([]);
export const canReopenClosedView = () => closedViews().length > 0;
/** What Reopen closed view would bring back, in words ("notes.md", "Subagents"), or null. */
export function closedViewLabel(): string | null {
  const t = closedViews()[closedViews().length - 1];
  if (!t) return null;
  if (t.kind === 'file') return decodeURIComponent(t.path.split('/').pop() || t.path).slice(0, 60);
  return t.kind === 'changes' ? 'Changes' : t.kind === 'visual' ? 'Canvas' : 'Closed tab';
}
/** Bring back the most recently closed tab; false when there is none. */
export function reopenClosedView(): boolean {
  const list = closedViews();
  const last = list[list.length - 1];
  if (!last) return false;
  setClosedViews(list.slice(0, -1));
  openPreview(last);
  return true;
}

/** Told when a Terminal tab is closed by hand (parity 7.2): the terminals
 *  store ends that chat's shells. Set by `store/terminals.ts` consumers, so
 *  this file stays free of imports; a tab dropped with its chat is not a close. */
let terminalTabClosed: ((chat: string) => void) | null = null;
export function onTerminalTabClosed(fn: ((chat: string) => void) | null): void { terminalTabClosed = fn; }

/** Closing a document also removes its historical occurrences. Other tabs stay. */
export function closePreviewDocument(key: string): void {
  previewIntent++;
  const matches = (t: PreviewTarget) => t.kind !== 'url' && previewDocumentKey(t) === key;
  const current = items()[at()];
  const docs = previewDocuments();
  const index = docs.findIndex(t => previewDocumentKey(t) === key);
  if (index < 0) return;
  batch(() => {
    const closed = docs[index];
    // A New tab is never kept, nor a Subagents tab: it belongs to one chat and would reopen under another.
    if (closed.kind === 'terminal') terminalTabClosed?.(closed.chat);
    if (closed.kind !== 'newtab' && closed.kind !== 'subagents' && closed.kind !== 'terminal') setClosedViews(list => [...list.filter(t => previewDocumentKey(t) !== key), closed].slice(-CLOSED_VIEW_CAP));
    const rest = docs.filter(t => !matches(t));
    setPreviewDocuments(rest);
    const kept = items().filter(t => !matches(t));
    setItems(kept);
    setAt(current && !matches(current) ? kept.indexOf(current) : kept.length - 1);
    if (current && matches(current)) {
      const next = rest[Math.min(index, rest.length - 1)];
      if (next) openPreview(next);
      else { setShown(false); setWebLoad(null); }
    }
  });
}

/** Hide the pane or restore it. A first open shows its Tools tab. */
export function togglePreview(chat?: string | null): void {
  previewIntent++;
  if (shown()) setShown(false);
  else if (items()[at()]) { setShown(true); setPreviewNonce(n => n + 1); }
  else openNewTab(chat);
}

let newTabs = 0;
/** The owner of a New tab opened on a chat not sent yet: no chat key
 *  matches it, so the next chat on screen (that one's first send included)
 *  drops it, and its tools never adopt another chat. */
export const DRAFT_TAB_OWNER = 'draft';
/** The tab strip's +: a New tab on `chat` (the chat on screen), selected. */
export function openNewTab(chat?: string | null): void {
  openPreview({ kind: 'newtab', id: ++newTabs, chat: chat || DRAFT_TAB_OWNER });
}

/** The New tab `key` is still open and still belongs to `chat`: checked
 *  after a tool's lookup, so a tab closed or a chat switched meanwhile
 *  opens nothing. */
export function newTabStillFor(key: string, chat: string): boolean {
  return previewDocuments().some(t => t.kind === 'newtab' && previewDocumentKey(t) === key && t.chat === chat);
}

/** After a New tab tool's lookup (`stamp` = `previewIntentNow()` taken before
 *  it): the tab is still open for `chat`, is still the selected tab of a shown
 *  pane, and no select, close, open or replace came meanwhile. Otherwise the
 *  lookup does nothing, so it never overrides a newer choice or reopens the pane. */
export function newTabLookupStillValid(key: string, chat: string, stamp: number): boolean {
  const t = previewTarget();
  return isLatestPreviewIntent(stamp) && newTabStillFor(key, chat) && !!t && t.kind === 'newtab' && previewDocumentKey(t) === key;
}

/** A New tab's tool picked: `target` takes the New tab's place in the strip
 *  and in history (Back still walks what came before it), and shows. A tool
 *  already open in another tab is selected there and the New tab goes. */
export function replacePreviewDocument(key: string, target: DocumentTarget): void {
  const next = clean(target) as DocumentTarget;
  const nextKey = previewDocumentKey(next);
  const docs = previewDocuments();
  // The New tab was closed (or dropped with its chat) meanwhile: nothing opens.
  if (!docs.some(t => previewDocumentKey(t) === key)) return;
  if (docs.some(t => previewDocumentKey(t) === nextKey)) {
    batch(() => { openPreview(next); closePreviewDocument(key); });
    return;
  }
  const matches = (t: PreviewTarget) => t.kind !== 'url' && previewDocumentKey(t) === key;
  batch(() => {
    setPreviewDocuments(list => list.map(t => matches(t) ? next : t));
    setItems(list => list.map(t => matches(t) ? next : t));
    openPreview(next);
  });
}

export function previewBack(): void {
  if (!canPreviewBack()) return;
  previewIntent++;
  const i = at() - 1;
  batch(() => { setAt(i); rememberDocument(items()[i]); askWebLoad(items()[i]); setPreviewNonce(n => n + 1); });
}

export function previewForward(): void {
  if (!canPreviewForward()) return;
  previewIntent++;
  const i = at() + 1;
  batch(() => { setAt(i); rememberDocument(items()[i]); askWebLoad(items()[i]); setPreviewNonce(n => n + 1); });
}

/** The page whose composer takes a draft made in the Preview pane (Ask the
 *  agent to fix this file), or null when one is on screen already: the pane
 *  opens beside any page, but only Conversation history (`chat`: `shownPage`
 *  names Library → Conversations so) and an open workspace's page have a
 *  composer (run 15 P1 review). */
export function composerPage(active: string, conversationInOpenWorkspace: boolean): 'chat' | 'today' | null {
  if (active === 'chat') return null;
  if (conversationInOpenWorkspace) return active === 'today' ? null : 'today';
  return 'chat';
}

/** Drop the items `goes` names from the tabs and from history; hides the
 *  pane when the shown item went. */
function dropWhere(goes: (t: PreviewTarget) => boolean): void {
  setPreviewDocuments(list => list.filter(t => !goes(t)));
  const list = items();
  if (!list.some(goes)) return;
  const current = list[at()];
  const kept = list.filter(t => !goes(t));
  batch(() => {
    setItems(kept);
    const index = current && !goes(current) ? kept.indexOf(current) : -1;
    setAt(index >= 0 ? index : kept.length - 1);
    if (!current || goes(current)) setShown(false);
  });
}

/** Drop items that only made sense in one conversation (its visuals), when
 *  another conversation comes on screen. Hides the pane if one was showing. */
export function dropConversationPreviews(): void {
  dropWhere(t => t.kind === 'visual');
}

/** Another chat came on screen: every Subagents tab whose chat isn't in
 *  `keep` (the open conversation's chain: itself, the one it continues in and
 *  the earlier ones, so a handover keeps the tab) goes, with its history
 *  (COO rebuild L3). */
export function dropSubagentPreviews(keep: ReadonlySet<string>): void {
  dropWhere(t => (t.kind === 'subagents' && !keep.has(t.parent)) || (t.kind === 'newtab' && !keep.has(t.chat)) || (t.kind === 'terminal' && !keep.has(t.chat)));
}

/** The chats the Subagents tabs (and New tabs, C5, and Terminal tabs, 7.2) belong to. */
export const subagentTabParents = (): string[] => previewDocuments().flatMap(t => t.kind === 'subagents' ? [t.parent] : t.kind === 'newtab' || t.kind === 'terminal' ? [t.chat] : []);

/** Tests only: back to a fresh launch. */
export function resetPreviewForTests(): void {
  batch(() => { setItems([]); setAt(-1); setShown(false); setPreviewNonce(0); setWebLoad(null); setPreviewDocuments([]); setClosedViews([]); });
}
