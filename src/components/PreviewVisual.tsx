import { Show, Switch, Match, createMemo, lazy } from 'solid-js';
import { EmptyArt } from './EmptyArt';
import { conversationCanvasScope } from '../store/workspaces';
import { chatVisuals, chatSessionLoading } from '../store/chat';
const ExpandedCanvas = lazy(() => import('./ExpandedCanvas').then(m => ({ default: m.ExpandedCanvas })));
const HtmlArtifact = lazy(() => import('./HtmlArtifact').then(m => ({ default: m.HtmlArtifact })));

/** A canvas or HTML visual of the open conversation in the Preview pane: a
 *  canvas's Expand shows it at full height (run 15 P1; it was the side
 *  panel's Visuals tab). Reads the conversation, never saves. */
export function PreviewVisual(props: { visualKey: string }) {
  // Keyed by the visual's key, not object identity, so a re-parse never
  // remounts the canvas (and loses its state).
  const visual = createMemo(() => chatVisuals().find(v => v.key === props.visualKey) ?? null, undefined, { equals: (a, b) => a?.key === b?.key });
  return <Show when={visual()} keyed fallback={
      <div class="preview-empty"><EmptyArt kind="visuals" evidence={{ ready: !chatSessionLoading() }}/><p>This visual belongs to a conversation that isn’t open now.</p></div>
    }>{v =>
      <div class="preview-visual">
        <Switch fallback={<div class="preview-visual-body"><HtmlArtifact lookWorkspace={conversationCanvasScope().workspaceId} code={v.code} lang={v.lang} defaultMode={conversationCanvasScope().workspaceId ? 'canvas' : 'offline'} /></div>}>
          <Match when={v.kind === 'canvas'}><ExpandedCanvas code={v.code} title={v.title} /></Match>
        </Switch>
      </div>
    }</Show>;
}
