import { Show } from 'solid-js';
import '../styles/thinking-row.css';

/** Provider-reported content only. Native disclosure starts closed. */
export function ThinkingRow(props: { text?: string }) {
  return <Show when={props.text?.trim()}><details class="chat-thinking-row"><summary>Thinking</summary><pre>{props.text}</pre></details></Show>;
}
