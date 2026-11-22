import { Show, createMemo } from 'solid-js';
import { importMermaid, flowHtml } from '../canvas/importFlow';
import { CodeBlock } from './CodeBlock';
import { HtmlArtifact } from './HtmlArtifact';
import { HtmlDoc } from './HtmlDoc';

/** Lazy host conversion; original source always remains available. Pane
 * previews use HtmlDoc, replies share HtmlArtifact's frame budget. */
export default function MermaidDiagram(props: { code: string; pane?: boolean; lookWorkspace?: string | null }) {
  const result = createMemo(() => importMermaid(props.code));
  const drawn = createMemo(() => { const r = result(); return r.ok ? r : null; });
  const reason = () => { const r = result(); return r.ok ? '' : `Not drawn: ${r.kind}. ${r.reason}`; };
  return <Show when={drawn()} fallback={<><CodeBlock code={props.code} lang="mermaid" /><p class="chat-canvas-note">{reason()}</p></>}>
    {r => <>
      <Show when={props.pane} fallback={<HtmlArtifact lookWorkspace={props.lookWorkspace} code={flowHtml(r())} lang="mermaid" defaultMode="offline" sourceView={{ text: props.code, lang: 'mermaid' }} />}>
        <HtmlDoc lookWorkspace={props.lookWorkspace} code={flowHtml(r())} defaultMode="offline" />
      </Show>
      <p class="chat-canvas-note">{r().note}</p>
    </>}
  </Show>;
}
