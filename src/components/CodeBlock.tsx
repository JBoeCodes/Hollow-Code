import { Show, createMemo, createSignal, onCleanup } from 'solid-js';
import { flashFooter } from '../store';
import { Highlight } from './Highlight';

// Wrap long lines (overnight 11-07, D1): one setting for every block, kept per
// machine. It lives here, not in `store/prefs.ts`, so the launch chunk (which
// has under 1 KB left of its budget) doesn't grow for a setting only a reply reads.
const WRAP_KEY = 'hollow.codeWrap';
const [wrapped, setWrappedSignal] = createSignal((() => { try { return localStorage.getItem(WRAP_KEY) === '1'; } catch { return false; } })());
function setWrapped(on: boolean) {
  setWrappedSignal(on);
  try { if (on) localStorage.setItem(WRAP_KEY, '1'); else localStorage.removeItem(WRAP_KEY); } catch { /* memory still holds it */ }
}

/**
 * A fenced code block lifted out of the markdown flow so it gets a
 * language label + a one-click copy button, and a Wrap toggle for long
 * lines. Rendered by ChatMessage for
 * every top-level ``` block that isn't an HTML artifact. The code text is
 * a Solid text node ({props.code}) so it's escaped — never innerHTML.
 */
export function CodeBlock(props: { code: string; lang?: string; initiallyExpanded?: boolean; onExpanded?: (value: boolean) => void }) {
  const [copied, setCopied] = createSignal(false);
  const [expanded, setExpanded] = createSignal(!!props.initiallyExpanded);
  const lines = createMemo(() => props.code.split('\n'));
  const shown = () => expanded() ? props.code : lines().slice(0, 24).join('\n');
  let copyTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(copyTimer));
  async function copy() {
    try {
      await navigator.clipboard.writeText(props.code);
      setCopied(true);
      flashFooter('Copied', 1000);
      clearTimeout(copyTimer);
      copyTimer = setTimeout(() => setCopied(false), 1200);
    } catch {
      flashFooter('× copy failed', 1500);
    }
  }
  return (
    <div class="chat-codeblock" classList={{ 'is-wrapped': wrapped() }}>
      <div class="chat-codeblock-head">
        <span class="chat-codeblock-lang">{props.lang || 'text'}</span>
        <span class="chat-codeblock-actions">
          {/* One setting for every block, so every block's toggle shows the same state. */}
          <button class="chat-codeblock-wrap" type="button" aria-pressed={wrapped()} aria-label="Wrap long lines"
            title={wrapped() ? 'Long lines wrap in every code block. Click to scroll them instead.' : 'Wrap long lines in every code block instead of scrolling them'} onClick={() => setWrapped(!wrapped())}>wrap</button>
          <button class="chat-codeblock-copy" type="button" aria-label={`Copy ${props.lang || 'text'} block`} onClick={copy}>
            {copied() ? 'copied' : 'copy'}
          </button>
        </span>
      </div>
      <pre class="chat-codeblock-pre"><code><Highlight code={shown()} lang={props.lang} /></code></pre>
      <Show when={lines().length > 24}>
        <button type="button" class="chat-codeblock-expand" aria-expanded={expanded()} onClick={() => {
          const next = !expanded(); setExpanded(next); props.onExpanded?.(next);
        }}>{expanded() ? 'Show fewer lines' : `Show all ${lines().length} lines`}</button>
      </Show>
    </div>
  );
}
