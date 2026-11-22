import { Show, createEffect, createMemo, createResource, createSignal, lazy, on, onCleanup, Suspense, untrack } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Select } from './Select';
import { Icon } from '../icons';
import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { HtmlDoc } from './HtmlDoc';
import { MediaPreview } from './MediaPreview';
import { NoteRenderer } from './NoteRenderer';
import { Spinner } from './Spinner';
import { Highlight } from './Highlight';
import { TablePreview } from './TablePreview';
import {
  chatArtifacts, chatPreviewPath, chatPreviewNonce, setChatPreviewNonce, chatPreviewLines,
  chatBusy, chatArtifactKind, type ChatArtifactKind,
  noteIndex, setActivePage, requestOpenVaultNote,
  askClaudeInChat, appendToChatDraft, flashFooter, chatAgentSessionKey, shownPage, chatCwd,
} from '../store';
import { closePreview, composerPage, openPreview } from '../store/preview';
import { windowAtMost } from '../windowLayout';
import { api } from '../api';
import { stripFrontmatter } from '../utils';
import { isInlineImage } from '../chatFileLinks';
import { fileMode } from '../canvas/canvasGrants';
import { conversationCanvasScope, workspaceDetail, workspaces } from '../store/workspaces';
import { vaultRoot } from '../store/chatRuntime';
import { filesPickerOptions, inVault, pinnedNotes, planFileOf, resultFileOf, vaultRelative } from '../projectFiles';
import { planRowsInOrder } from '../planModel';
import { sha256Hex } from '../sectionMarkdown';
import { errMsg } from '../utils';
import { opensInQuickTime } from '../mediaKinds';
import { ErrorDetails, StatePanel } from './ui';
import { unlessErrored } from '../stableResource';
import { previewFailure } from '../filePreviewFailure';
import { highlights, prettyJson, textViewOf } from '../previewKinds';
import { fileDetailsLine, type PreviewFileFacts } from '../previewDetails';
import { LoadingProgress } from './MotionKit';

const MermaidDiagram = lazy(() => import('./MermaidDiagram'));
const PdfPreview = lazy(() => import('./PdfPreview').then(m => ({ default: m.PdfPreview })));
const OfficePreview = lazy(() => import('./OfficePreview').then(m => ({ default: m.OfficePreview })));
const PreviewSourceLines = lazy(() => import('./PreviewSourceLines'));
// Line-anchored review notes on a text file (row OC9): loads with its first use.
const PreviewReviewNotes = lazy(() => import('./PreviewReviewNotes'));
// A Result's review bar (COO rebuild L4) loads with the first Result opened.
const ResultReviewBar = lazy(() => import('./ResultReviewBar').then(m => ({ default: m.ResultReviewBar })));
/** The Files picker's entry for a build workspace's Changes tab. */
const CHANGES_OPTION = 'hollow:changes';

/** Kinds that have a rendered "Preview" mode besides their source. */
const RENDERED_KINDS: ChatArtifactKind[] = ['html', 'markdown', 'svg', 'image', 'diagram'];

const KIND_LABEL: Record<ChatArtifactKind, string> = {
  html: 'html',
  markdown: 'markdown',
  svg: 'svg',
  image: 'image',
  video: 'video',
  pdf: 'PDF',
  diagram: 'diagram',
  code: 'text',
  binary: 'file',
};
/**
 * A file in the Preview pane (run 15 P1; it was the side panel's Files view).
 * HTML renders as a live sandboxed document, Markdown rendered, SVG/images
 * as pictures, videos through `hollow-media`, CSV/TSV as a table, JSON
 * pretty-printed and code highlighted, with a Preview/Source toggle, the
 * conversation's files in a picker, refresh, open-externally and reveal, and
 * an "Ask the agent to fix" affordance that hands rendering errors (captured
 * inside the sandboxed iframe) back to the agent. PreviewPane.tsx owns the
 * pane around it: back/forward, the tabs, close, width.
 *
 * UI clutter pass (2026-11-06): the file's own controls sit in the pane's one
 * header row (`toolbar`, a slot beside the tabs), so the content starts right
 * under it. The tab names the file; its kind, path, size and time are the
 * tab's tooltip (`onFacts` hands the size and time up) and rows of the •••
 * menu, beside Copy path.
 */
export function ChatPreview(props: { toolbar: HTMLElement | undefined; onFacts?: (facts: PreviewFileFacts | null) => void }) {
  const [view, setView] = createSignal<'preview' | 'source'>('preview');
  const [jsErrors, setJsErrors] = createSignal<string[]>([]);
  // Review notes (row OC9): whether the text file shows as numbered lines with notes.
  // It stays on while another reviewable file opens; only a file it can't number hides it.
  const [reviewing, setReviewing] = createSignal(false);

  const artifact = createMemo(() => {
    const p = chatPreviewPath();
    if (!p) return null;
    if (isInlineImage(p)) return { path: p, name: 'Image', kind: 'image' as const, touchedAt: 0 };
    return chatArtifacts().find(a => a.path === p)
      ?? { path: p, name: p.slice(p.lastIndexOf('/') + 1), kind: chatArtifactKind(p), touchedAt: 0 };
  });
  // An attached or generated picture: shown from its data URL, with no file
  // on disk to open, reveal or hand back to the agent.
  const inline = () => isInlineImage(chatPreviewPath() ?? '');
  // The vault's path, to tell a project's notes, its Results and its PLAN.md.
  const [root] = createResource(() => vaultRoot());
  const rel = () => vaultRelative(chatPreviewPath() ?? '', root.latest);
  // A file in a workspace's outputs/ opens with its review bar (COO rebuild
  // L4: it was the Results section); a workspace's PLAN.md ticks its boxes.
  const resultRef = createMemo(() => resultFileOf(rel(), workspaces()), null, { equals: (a, b) => a?.path === b?.path && a?.workspaceId === b?.workspaceId });
  const planWorkspace = () => planFileOf(rel(), workspaces());
  const [compare, setCompare] = createSignal(false);
  createEffect(on(() => chatPreviewPath(), () => setCompare(false), { defer: true }));
  // A build workspace's changed files, for the picker's Changed files group
  // (COO rebuild L4: it was the Changes section's list), read again when
  // another file opens.
  const build = () => { const w = workspaceDetail()?.workspace; return w?.build?.exists ? w.id : null; };
  const [changes] = createResource(() => { const id = build(); return id ? { id, path: chatPreviewPath() } : null; },
    ({ id }) => api.buildStatus(id).catch(() => null));
  // The open project's notes pinned at the top, its changed files, then the
  // conversation's files, plus a file opened that the agent only read (it is
  // not one of them) so the picker can name it.
  const pickerOptions = createMemo(() => {
    const opts = chatArtifacts().map(a => ({ value: a.path, label: a.name, detail: a.path.includes('/') ? a.path.replace(/\/[^/]+$/, '') : undefined }));
    const current = artifact();
    if (current && !opts.some(o => o.value === current.path)) {
      opts.unshift({ value: current.path, label: current.name, detail: inline() ? 'from the conversation' : current.path.replace(/\/[^/]+$/, '') });
    }
    const vault = root.latest;
    const d = workspaceDetail();
    const notes = vault && d ? pinnedNotes(d.workspace, d.files).map(p => inVault(vault, p)) : [];
    const status = build() ? changes.latest : null;
    const changed = status ? status.files.filter(f => f.status !== 'deleted').map(f => inVault(status.worktree, f.path)) : [];
    const all = filesPickerOptions({ notes, changed, conversation: opts });
    return build() ? [...all, { value: CHANGES_OPTION, label: 'All changes: commit and publish…', group: 'Changed files' }] : all;
  });
  const pickFile = (value: string) => {
    const id = build();
    if (value === CHANGES_OPTION) { if (id) openPreview({ kind: 'changes', workspaceId: id }); }
    else openPreview({ kind: 'file', path: value });
  };

  // Re-read on path change and on every nonce bump (live agent edits, the
  // refresh button, opening it again). Collected JS errors reset each load.
  const [file] = createResource(
    () => {
      const p = chatPreviewPath();
      return p ? { path: p, nonce: chatPreviewNonce() } : null;
    },
    async (src: { path: string; nonce: number }) => {
      setJsErrors([]);
      if (isInlineImage(src.path)) {
        return { previewPath:src.path, previewNonce:src.nonce, kind: 'image', text: null, dataUrl: src.path, size: Math.round(src.path.length * 0.75), modifiedMs: null, truncated: false };
      }
      return { ...await api.chatPreviewFile(src.path), previewPath:src.path, previewNonce:src.nonce };
    },
  );

  // Every read goes through `loaded`: an errored resource throws when read,
  // and that took the whole page into its recovery boundary ("Your work
  // could not load", 10-23) instead of saying here that the file failed.
  const loaded = () => unlessErrored(file);
  // The read's real size and time, for the tab's tooltip; an attached picture
  // has no file on disk, so nothing.
  const facts = createMemo((): PreviewFileFacts | null => {
    const f = loaded();
    return f && !inline() && f.previewPath === chatPreviewPath() ? { path: f.previewPath, size: f.size, modifiedMs: f.modifiedMs } : null;
  }, null, { equals: (a, b) => a?.path === b?.path && a?.size === b?.size && a?.modifiedMs === b?.modifiedMs });
  createEffect(() => props.onFacts?.(facts()));
  // Shown no more (a web page or canvas tab took over): no tab keeps a size
  // and time that could go stale.
  onCleanup(() => props.onFacts?.(null));
  const lineTarget = () => {
    const target = chatPreviewLines();
    const value = loaded();
    return target && target.path === chatPreviewPath() && value?.previewPath === target.path && value.previewNonce === chatPreviewNonce() && !file.loading ? target : null;
  };
  let viewBeforeCite: 'preview' | 'source' | null = null;
  createEffect(() => {
    if (chatPreviewLines()) {
      viewBeforeCite ??= untrack(view);
      if (lineTarget()) { setView('source'); setReviewing(false); }
    } else if (viewBeforeCite !== null) {
      setView(viewBeforeCite);
      viewBeforeCite = null;
    }
  });
  const failure = createMemo(() => file.error && !file.loading ? previewFailure(file.error) : null);
  const kind = () => artifact()?.kind ?? 'code';
  const textView = createMemo(() => textViewOf(chatPreviewPath() ?? ''));
  // Tables and JSON have a rendered view of their own beside the source.
  const rendered = () => (RENDERED_KINDS.includes(kind()) || (kind() === 'code' && textView().kind !== 'code')) && loaded()?.kind !== 'binary';
  const showToggle = () => rendered() && kind() !== 'image';
  // Plain text and Markdown files only: never a picture, PDF, canvas, web page,
  // HTML, diagram, table or JSON (those have views of their own).
  const reviewable = () => !inline() && loaded()?.kind === 'text' && loaded()?.previewPath === chatPreviewPath()
    && (kind() === 'markdown' || (kind() === 'code' && textView().kind === 'code'));
  const notesOn = () => reviewing() && reviewable();

  function refresh() { setChatPreviewNonce(n => n + 1); }

  // PLAN.md's checkboxes tick in the Markdown view (COO rebuild L4: the Plan
  // section's rows did): the write is refused, and nothing ticked, when the
  // row or the file moved on since it was shown (`toggle_plan_row`).
  const [ticking, setTicking] = createSignal(false);
  const planTasks = () => {
    const id = planWorkspace();
    const f = loaded();
    if (!id || !f || f.truncated || f.text == null || f.previewPath !== chatPreviewPath()) return undefined;
    const text = f.text;
    const rows = planRowsInOrder(text);
    return { count: rows.length, busy: ticking(), toggle: (index: number, done: boolean) => void tick(id, text, rows[index], done) };
  };
  async function tick(workspaceId: string, text: string, row: ReturnType<typeof planRowsInOrder>[number] | undefined, done: boolean) {
    if (!row || ticking()) return;
    setTicking(true);
    try {
      await api.togglePlanRow({ workspaceId, line: row.id.line, lineSha256: row.id.sha256, expectedRevision: sha256Hex(text), done });
    } catch (e) {
      const msg = errMsg(e);
      flashFooter(msg.startsWith('PLAN_CHANGED') ? 'PLAN.md changed since it was shown, so nothing was ticked. It shows as it is now.' : `× Couldn’t tick that row: ${msg}`, 4000);
    } finally { setTicking(false); refresh(); }
  }

  async function openExternal(reveal: boolean, withApp?: 'quicktime') {
    const p = chatPreviewPath();
    if (!p) return;
    try {
      // An open the backend refused to launch (an app, executable or script,
      // even behind a link) was revealed instead; say so (H4).
      const how = await api.chatPreviewOpen(p, reveal, withApp);
      if (how === 'revealed' && !reveal) flashFooter('Shown in Finder: double-click it there to open it (it could run code)', 5000);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      flashFooter(`× open failed: ${msg}`, 2500);
    }
  }

  async function copyPath() {
    const p = chatPreviewPath();
    if (!p) return;
    try {
      await navigator.clipboard.writeText(p);
      flashFooter('Path copied', 1200);
    } catch {
      flashFooter('× copy failed', 1500);
    }
  }

  // `<a data-vault="Name">` inside a rendered HTML artifact, or a [[wikilink]]
  // in rendered markdown — jump to that note in the Vault tab.
  function openInVault(name: string) {
    const match = noteIndex().find(n => n.basename.toLowerCase() === name.toLowerCase());
    if (match) {
      setActivePage('vault');
      requestOpenVaultNote(match.path);
    } else {
      flashFooter(`No vault note named "${name}"`, 2500);
    }
  }

  function onJsError(msg: string) {
    setJsErrors(prev => (prev.includes(msg) || prev.length >= 8 ? prev : [...prev, msg]));
  }

  // Hand the artifact back to the agent for troubleshooting. Prefilled (never
  // auto-sent) so the user reviews before spending tokens; includes the
  // runtime errors the sandbox caught so the agent has a real signal.
  async function askToFix() {
    const a = artifact();
    if (!a) return;
    const errs = jsErrors();
    const errPart = errs.length
      ? `\n\nRendering it surfaced ${errs.length === 1 ? 'this runtime error' : 'these runtime errors'}:\n${errs.map(e => `- ${e}`).join('\n')}`
      : '';
    // The pane opens beside any page, but only Conversation history and an
    // open workspace's page have a composer: go to the open conversation's
    // page first (run 15 P1 review), and only once that navigation went
    // ahead (an unsaved editor may ask first; Cancel queues nothing). In a
    // window too narrow for both, the pane then steps aside so the draft is
    // on screen.
    const inWorkspace = !!chatCwd() && workspaceDetail()?.workspace.cwd === chatCwd();
    const page = composerPage(shownPage(), inWorkspace);
    if (page) {
      await setActivePage(page);
      if (shownPage() !== page) return;
    }
    askClaudeInChat(
      `Review ${a.path} — Read it back, verify the ${KIND_LABEL[a.kind]} is well-formed and renders/reads the way you intended, and fix any issues you find.${errPart}`,
    );
    if (windowAtMost('medium')) closePreview();
  }

  // Review notes go to the chat as one draft message. Like "Ask the agent to
  // fix", a composer comes on screen first; nothing is sent.
  async function sendNotes(text: string): Promise<boolean> {
    const inWorkspace = !!chatCwd() && workspaceDetail()?.workspace.cwd === chatCwd();
    const page = composerPage(shownPage(), inWorkspace);
    if (page) {
      await setActivePage(page);
      if (shownPage() !== page) return false;
    }
    appendToChatDraft(text);
    if (windowAtMost('medium')) closePreview();
    return true;
  }

  const svgDataUrl = createMemo(() => {
    const text = loaded()?.text;
    if (!text) return null;
    try {
      return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(text)))}`;
    } catch {
      return null;
    }
  });

  /** Plain source, highlighted when the pane knows the language. */
  const source = (text: string) => {
    const tv = textView();
    const lang = tv.kind === 'json' ? 'json' : tv.kind === 'code' ? tv.lang : undefined;
    return <pre class="chat-preview-source"><code><Show when={highlights(text, lang)} fallback={text}><Highlight code={text} lang={lang} /></Show></code></pre>;
  };

  // The rows are made again only for another file: a live edit's new size or
  // time updates the details row in place (its label is a getter), so a row
  // focused by keyboard keeps focus while the agent writes (review of #1180).
  const fileActions = createMemo((): ActionMenuItem[] => {
    const path = chatPreviewPath() ?? '';
    return [
      ...(kind() === 'video' && opensInQuickTime(path)
        ? [{ label: 'Open in QuickTime Player', action: () => void openExternal(false, 'quicktime') }]
        : []),
      { label: 'Open in default app', action: () => void openExternal(false) },
      { label: 'Reveal in Finder', action: () => void openExternal(true) },
      { label: 'Copy path', action: () => void copyPath() },
      { get label() { return fileDetailsLine(kind(), facts()); }, action: () => {}, informational: true, separatorBefore: true },
      { label: path, action: () => {}, informational: true },
    ];
  });

  // The file's controls, in the pane's header row: whether the agent is still
  // working on it, the conversation's other files, the view, refresh, the
  // fix request and the rarer actions with the file's details.
  const tools = () => (
    <div class="chat-preview-actions">
      <Show when={chatBusy()}>
        <span class="live-dot on" title="Agent is still working: the preview refreshes as it edits" aria-hidden="true" />
      </Show>
      <Show when={pickerOptions().length > 1}>
        <span class="chat-preview-files" title="Files: the project's notes, its changed files and this conversation's files">
          <Select quiet size="sm" class="chat-preview-picker" label="Files" value={chatPreviewPath() ?? ''} onChange={pickFile}
            options={pickerOptions()} align="end" trigger={() => <Icon name="folder" size={14} />} />
        </span>
      </Show>
      <Show when={showToggle()}>
        {/* A real segmented control (asks 10-11 A3): it had no styles of its own
            and read as two disabled buttons. */}
        <div class="ui-segmented chat-preview-toggle" role="group" aria-label="Show the file as">
          <button type="button" aria-label="Preview" title="Preview" aria-pressed={view() === 'preview' && !notesOn()} onClick={() => { setView('preview'); setReviewing(false); }}><Icon name="eye" size={14}/></button>
          <button type="button" aria-label="Source" title="Source" aria-pressed={view() === 'source' && !notesOn()} onClick={() => { setView('source'); setReviewing(false); }}><Icon name="fileCode" size={14}/></button>
        </div>
      </Show>
      <Show when={reviewable()}>
        <button type="button" class="btn ghost small chat-preview-notes-toggle" aria-pressed={reviewing()} onClick={() => setReviewing(v => !v)}
          title="Leave notes on lines of this file, then send them all to the chat as one message">
          <Icon name="message" size={13} /> Review notes
        </button>
      </Show>
      <Show when={!inline()}>
        <button class="chat-panel-iconbtn" title="Reload from disk" aria-label="Refresh preview" onClick={refresh}>
          <Icon name="refresh" size={13} />
        </button>
        <button class="chat-panel-iconbtn" title="Ask the agent to review and fix this file (fills the message box)" aria-label="Ask the agent to fix this file" onClick={() => void askToFix()}>
          <Icon name="wand" size={13} />
        </button>
        {/* The rarer ones in the file's own menu (asks 10-11 A3), then what
            the header line used to say: kind, size, time and the full path. */}
        <ActionMenu label="File actions" items={fileActions()} />
      </Show>
    </div>
  );

  return (
    <div class="chat-preview" aria-label="File preview">
      <Show when={props.toolbar}>{slot => <Portal mount={slot()}>{tools()}</Portal>}</Show>

      <Show when={jsErrors().length > 0}>
        <button type="button" class="chat-preview-errors" onClick={() => void askToFix()} title="Runtime errors from the rendered document: click to hand them to your agent">
          <Icon name="alert" size={12} />
          {jsErrors().length} script error{jsErrors().length === 1 ? '' : 's'}: ask the agent to fix
        </button>
      </Show>

      <Show when={resultRef()} keyed>{ref =>
        <Suspense><ResultReviewBar workspaceId={ref.workspaceId} path={ref.path} nonce={chatPreviewNonce()} current={loaded()?.previewPath === chatPreviewPath() ? loaded()?.text ?? '' : ''} compare={compare()} onCompare={setCompare} /></Suspense>
      }</Show>

      {/* A scroll region reachable by keyboard (axe scrollable-region-focusable):
          a Markdown file with no links has nothing else to take focus. */}
      <div class="chat-preview-body viewer" classList={{ 'pdf-document': kind() === 'pdf' }} hidden={!!resultRef() && compare()}
        tabIndex={0} role="region" aria-label={`${artifact()?.name ?? 'File'} contents`}>
        {/* `file()` keeps the previous value while a refetch is in flight, so
            live agent-edit refreshes swap content in place instead of
            flashing a spinner — the spinner shows on the first load,
            a different file, and a Retry after a failure. */}
        <Show when={loaded()?.previewPath === chatPreviewPath() || !file.loading} fallback={<div class="chat-preview-loading"><LoadingProgress loading label="Loading preview" /><Spinner /></div>}>
          <Show
            when={!failure()}
            fallback={<Show when={failure()}>{f =>
              <StatePanel compact kind="failed" class="chat-preview-failed" title={`Couldn’t open ${artifact()?.name ?? 'this file'}`}
                actions={<>
                  <button type="button" class="btn small" onClick={refresh}>Retry</button>
                  <Show when={f().kind === 'too-large'}><button type="button" class="btn small" onClick={() => void openExternal(false)}>Open in default app</button></Show>
                  <Show when={f().kind !== 'missing'}><button type="button" class="btn small ghost" onClick={() => void openExternal(true)}>Show in Finder</button></Show>
                </>}>{f().text}<ErrorDetails detail={f().detail} /></StatePanel>
            }</Show>}
          >
            <Show when={loaded()?.previewPath === chatPreviewPath() ? loaded() : null}>
              {(f) => (
                <>
                  {/* image (always rendered; no source mode) */}
                  <Show when={f().kind === 'image'}>
                    <img class="chat-preview-img" src={f().dataUrl ?? ''} alt={artifact()?.name ?? 'image'} />
                  </Show>

                  {/* video — plays here through the hollow-media scheme */}
                  {/* `kind()` follows the path at once while `f()` still holds the
                      last file, so a switch away never grants the new path. */}
                  <Show when={f().kind === 'video' && kind() === 'video'}>
                    <MediaPreview variant="pane" path={chatPreviewPath()!} nonce={chatPreviewNonce()} sessionKey={chatAgentSessionKey()} />
                  </Show>

                  <Show when={f().kind === 'pdf' && kind() === 'pdf'}>
                    <Suspense fallback={<Spinner />}><PdfPreview path={chatPreviewPath()!} nonce={chatPreviewNonce()} sessionKey={chatAgentSessionKey()} /></Suspense>
                  </Show>

                  {/* binary: Word, Excel and PowerPoint text (B1b), else details only */}
                  <Show when={f().kind === 'binary'}>
                    <Suspense fallback={<Spinner />}><OfficePreview path={chatPreviewPath()!} nonce={chatPreviewNonce()} sessionKey={chatAgentSessionKey()} onOpen={openExternal} /></Suspense>
                  </Show>

                  {/* text-backed kinds */}
                  <Show when={f().kind === 'text'}>
                    <Show when={f().truncated}>
                      <p class="hint chat-preview-hint">(showing the first 1.5 MB — open externally for the full file)</p>
                    </Show>
                    <Show when={notesOn()}>
                      <Suspense fallback={<Spinner />}>
                        <PreviewReviewNotes path={chatPreviewPath()!} text={f().text ?? ''} truncated={f().truncated} onSend={sendNotes} />
                      </Suspense>
                    </Show>
                    <Show when={!notesOn()}>
                    <Show when={view() === 'preview' && kind() === 'html'}>
                      <HtmlDoc lookWorkspace={conversationCanvasScope().workspaceId} code={stripFrontmatter(f().text ?? '')} onNavigate={openInVault} onJsError={onJsError}
                        defaultMode={fileMode(chatPreviewPath() ?? '', chatArtifacts().some(a => a.path === chatPreviewPath()))} grantKey={`file:${chatPreviewPath() ?? ''}`} />
                    </Show>
                    <Show when={view() === 'preview' && kind() === 'markdown'}>
                      <NoteRenderer content={f().text ?? ''} onWikilinkClick={openInVault} tasks={planTasks()} />
                    </Show>
                    <Show when={view() === 'preview' && kind() === 'diagram'}>
                      <Suspense fallback={<pre class="chat-preview-source">{f().text ?? ''}</pre>}><MermaidDiagram lookWorkspace={conversationCanvasScope().workspaceId} code={f().text ?? ''} pane /></Suspense>
                    </Show>
                    <Show when={view() === 'preview' && kind() === 'svg' && svgDataUrl()}>
                      <img class="chat-preview-img" src={svgDataUrl()!} alt={artifact()?.name ?? 'svg'} />
                    </Show>
                    <Show when={view() === 'preview' && kind() === 'code' && textView().kind === 'table'}>
                      <TablePreview text={f().text ?? ''} name={artifact()?.name ?? ''} />
                    </Show>
                    <Show when={view() === 'preview' && kind() === 'code' && textView().kind === 'json'}>
                      {source(prettyJson(f().text ?? '') ?? f().text ?? '')}
                    </Show>
                    <Show when={view() === 'source' || (kind() === 'code' && textView().kind === 'code') || (kind() === 'svg' && !svgDataUrl())}>
                      <Show when={lineTarget()} fallback={source(f().text ?? '')}>{target =>
                        <Suspense fallback={<pre class="chat-preview-source">{f().text ?? ''}</pre>}>
                          <PreviewSourceLines text={f().text ?? ''} line={target().line} end={target().end} truncated={f().truncated} />
                        </Suspense>
                      }</Show>
                    </Show>
                    </Show>
                  </Show>
                </>
              )}
            </Show>
          </Show>
        </Show>
      </div>
    </div>
  );
}
