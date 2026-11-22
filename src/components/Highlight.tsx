import { For, createMemo } from 'solid-js';
import { tokenize } from '../highlight';

/**
 * Renders syntax-highlighted code as escaped text nodes + <span class="tok-*">
 * wrappers — never innerHTML, so it's safe on arbitrary/agent-authored code.
 * Drop it inside a <code>/<pre>; it emits inline content only.
 */
export function Highlight(props: { code: string; lang?: string }) {
  const toks = createMemo(() => tokenize(props.code, props.lang));
  return (
    <For each={toks()}>
      {(tk) => (tk.t === 'plain' ? tk.v : <span class={`tok-${tk.t}`}>{tk.v}</span>)}
    </For>
  );
}
