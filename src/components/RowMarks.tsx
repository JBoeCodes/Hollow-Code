import { For } from 'solid-js';
import { chatRowMarks } from '../store/rowStatus';
import type { AgentSessionSummary } from '../types';
import '../styles/row-marks.css';

/** The status marks at the end of a chat row: ask (Approval or Input), Plan, an
 *  unread dot and the linked pull request. Each says its words to a screen reader. */
export function RowMarks(props: { session: AgentSessionSummary; here: boolean; merged?: { number: number } | null }) {
  const marks = () => chatRowMarks(props.session, props.here, props.merged);
  return <span class="work-row-marks"><For each={marks()}>{m =>
    <span class="work-row-mark" data-mark={m.id} data-tone={m.tone} role="img" aria-label={m.label} title={m.label}>{m.text}</span>
  }</For></span>;
}
