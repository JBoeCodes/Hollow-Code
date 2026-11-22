import { Index, Show, createEffect, createMemo, createSignal, onCleanup, lazy, Suspense } from 'solid-js';
import { CodeBlock } from './CodeBlock';
import { MarkdownView } from './MarkdownView';
import { HtmlArtifact } from './HtmlArtifact';
import { ChatCanvas, type CanvasAnswerHandler } from './ChatCanvas';
import { htmlFenceMode } from '../canvas/chatCanvas';
import { conversationCanvasScope } from '../store/workspaces';
import { parseSegments, reuseFinishedCanvases, reuseSettledSegments, type Segment } from '../chatSegments';

const MermaidDiagram = lazy(() => import('./MermaidDiagram'));
const ReplyMarkdown = lazy(() => import('./ReplyMarkdown'));

// Preserve prose geometry while the reply-only decorator loads, without
// letting raw local/remote image URLs or links reach the browser meanwhile.
function pendingReplyImages(html: string): string {
  const template = document.createElement('template');
  template.innerHTML = html;
  for (const img of template.content.querySelectorAll('img')) {
    const placeholder = document.createElement('span');
    placeholder.textContent = img.getAttribute('alt') || 'Loading image…';
    img.replaceWith(placeholder);
  }
  return template.innerHTML;
}

/**
 * Renders one assistant/system chat turn. While the turn is still streaming
 * (`streaming`), HTML fences render as plain code (you watch the source build
 * up) and only become live artifacts once the turn is finalized — avoids
 * re-mounting an iframe on every token. An old whiteboard fence (removed
 * 2026-10-07) is a notice and its JSON as code (`chatSegments`).
 * A hollow-canvas fence shows a skeleton once its root tag closes and the
 * canvas itself as soon as its fence ends, even mid-turn (`ChatCanvas`).
 *
 * Html previews and canvases render in place in every host (Canvas C3, wave
 * 2 C1). In a
 * quick or workspace chat an html fence opens in Canvas mode, elsewhere
 * Offline; a canvas always opens in Canvas mode.
 */
export function ChatMessage(props: {
  text: string;
  role: 'assistant' | 'system';
  streaming?: boolean;
  /** The reply belongs to the run that is live now. A reply that arrives
   *  whole (Codex's agent messages, a Claude stream without deltas) is never
   *  `streaming`, but its canvases are still this turn's and are saved. */
  liveRun?: boolean;
  /** The history run id of the turn this reply belongs to: a canvas saved
   *  from it is recorded against that turn (a curated section canvas binds
   *  to its start, Canvas C6 · K1). */
  runId?: string | null;
  /** Show the streaming caret (see `streamCaret.ts`); separate from
   * `streaming`, which also governs how fences render mid-stream. */
  caret?: boolean;
  class?: string;
  onAsk?: (prompt: string) => void;
  /** An ask-canvas's answers, after the person pressed Hollow's Send under
   *  it (question cards, 2026-10-14): sent as their next message. */
  onCanvasAnswer?: CanvasAnswerHandler;
}) {
  // While streaming, throttle the segment re-parse (and the downstream
  // marked + DOMPurify re-render of every segment) to ~12fps instead of
  // once per delta — on a long answer the per-token full-message re-render
  // is what makes the composer feel sluggish. The final text always renders
  // immediately once streaming flips off.
  const [parsedText, setParsedText] = createSignal(props.text);
  let throttle: ReturnType<typeof setTimeout> | null = null;
  createEffect(() => {
    const t = props.text;
    if (!props.streaming) {
      if (throttle) { clearTimeout(throttle); throttle = null; }
      setParsedText(t);
      return;
    }
    if (throttle) return;
    throttle = setTimeout(() => {
      throttle = null;
      setParsedText(props.text);
    }, 80);
  });
  onCleanup(() => { if (throttle) clearTimeout(throttle); });

  // Whether this reply has streamed, or arrived during its live run, in this
  // mount: a canvas that becomes ready then is saved; one rendered from a
  // finished transcript only reads.
  const [live, setLive] = createSignal(!!props.streaming || !!props.liveRun);
  createEffect(() => { if (props.streaming || props.liveRun) setLive(true); });
  // Read once per mount: the conversation (and so its chat kind) is fixed.
  const htmlMode = htmlFenceMode(!!conversationCanvasScope().workspaceId);

  // Throttling alone still rebuilt the whole answer 12x/sec: parseSegments
  // hands back all-new objects and `<For>` keys by reference, so every row was
  // disposed and recreated, re-running marked + DOMPurify (and the highlighter)
  // over prose that had settled long ago. Reusing the previous object for each
  // unchanged leading segment keeps those rows mounted, so a flush only costs
  // the growing tail.
  //
  // Reading `props.streaming` inside the memo is load-bearing, not incidental.
  // It bounds reuse to the streaming window, and — because `finalizePartials`
  // flips only `partial`, never `text` — it is what guarantees the memo re-runs
  // when the turn settles. That run reuses nothing but finished canvases, so
  // every other row is recreated exactly once with `streaming === false`,
  // which is what lets a streamed ```html/```svg fence upgrade from source to
  // the live sandboxed artifact below. Without that read the rows kept their streaming-time identity and a
  // finished artifact could stay stuck rendering as a code block.
  // Markdown rows are the exception (gap 8): they stay mounted by position and
  // only their changed blocks are replaced, during the stream and at its end.
  let prevSegs: Segment[] = [];
  // A growing code segment is a new object each flush, so its row is rebuilt;
  // the reply owns its disclosure.
  const expandedCode = new Map<number, boolean>();
  // A table still being written is rebuilt with its block each flush, so the
  // reply also owns where each segment's tables were scrolled (`keepTableScroll`).
  const tableScroll = new Map<number, number[]>();
  const scrollOf = (i: number) => tableScroll.get(i) ?? (tableScroll.set(i, []), tableScroll.get(i)!);
  const segs = createMemo(() => {
    const next = parseSegments(parsedText());
    prevSegs = props.streaming ? reuseSettledSegments(prevSegs, next) : reuseFinishedCanvases(prevSegs, next);
    return prevSegs;
  });

  return (
    <div
      class={`chat-msg ${props.class ?? ''}`}
      classList={{ streaming: !!props.streaming, typing: !!props.caret }}
    >
      {/* Rows by position (overnight 11-07, D1, gap 8): a Markdown row stays
          mounted while its text grows, and `ReplyMarkdown` swaps only the
          blocks that changed; every other row is rebuilt when its segment
          object changes, as the keyed list did. */}
      <Index each={segs()}>
        {(item, index) => {
          const md = () => { const seg = item(); return seg.kind === 'md' ? seg : null; };
          return <Show when={md()} fallback={<Show when={item()} keyed>{seg => {
            // Html previews and canvases render in place (Canvas C3).
            if (seg.kind === 'canvas') {
              return <ChatCanvas code={seg.code} closed={seg.closed} settled={!props.streaming} live={live()} turnId={props.runId ?? null} onAsk={props.onAsk} onAnswer={props.onCanvasAnswer} />;
            }
            if (seg.kind === 'html' && !props.streaming) {
              return <HtmlArtifact lookWorkspace={conversationCanvasScope().workspaceId} code={seg.code} lang={seg.lang} defaultMode={htmlMode} />;
            }
            if (seg.kind === 'code' && seg.lang === 'mermaid' && !props.streaming) return <Suspense fallback={<CodeBlock code={seg.code} lang={seg.lang} />}><MermaidDiagram lookWorkspace={conversationCanvasScope().workspaceId} code={seg.code} /></Suspense>;
            return seg.kind === 'md' ? null : <CodeBlock code={seg.code} lang={seg.lang} initiallyExpanded={expandedCode.get(index)} onExpanded={value => expandedCode.set(index, value)} />;
          }}</Show>}>
            {seg => <Suspense fallback={<MarkdownView text={seg().text} class={`reply-loading ${props.role === 'system' ? 'system' : ''}`} forbidMedia transform={pendingReplyImages} onLinkClick={e => { e.preventDefault(); e.stopPropagation(); }} />}><ReplyMarkdown text={seg().text} class={props.role === 'system' ? 'system' : ''} scroll={scrollOf(index)} /></Suspense>}
          </Show>;
        }}
      </Index>
    </div>
  );
}

