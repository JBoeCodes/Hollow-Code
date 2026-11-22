import { Show } from 'solid-js';
import { flashFooter } from '../store';

export default function ReceiptCheckActions(props: { command: string; onAskRerun?: (text: string) => void }) {
  const shortened = () => [...props.command].length >= 320 && props.command.endsWith('…');
  const display = () => [...props.command].length > 321 ? [...props.command].slice(0, 320).join('') + '…' : props.command;
  async function copy() {
    try { await navigator.clipboard.writeText(props.command); flashFooter('Recorded command copied', 1500); }
    catch { flashFooter('The command could not be copied. Select its text to copy it.', 3500); }
  }
  return <>
    <code class="toolcall-result">{display()}</code>
    <Show when={shortened()}><span class="toolcall-extra">Shortened when recorded; the original command is unavailable.</span></Show>
    <div class="receipt-check-actions">
      <button type="button" class="btn ghost sm" onClick={() => void copy()}>{shortened() ? 'Copy recorded command (cut short)' : 'Copy command'}</button>
      <Show when={props.onAskRerun}>
        <button type="button" class="btn ghost sm" onClick={() => props.onAskRerun?.(`Review and rerun this recorded check${shortened() ? ' (the record was shortened; recover the complete command first)' : ''}:\n\n${props.command}`)}>Ask to rerun</button>
      </Show>
    </div>
  </>;
}
