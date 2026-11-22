import { Show } from 'solid-js';
import { AgentMark } from './AgentMark';
import { openPreview } from '../store/preview';
import { chatAgentSessionKey } from '../store/chatRuntime';
import { createSecondClock, createSubagentsRead } from '../store/subagents';
import { durationWords, findRow, liveWorkingMs, statusWords, subagentCall, tickFrom } from '../subagentsModel';
import '../styles/subagents.css';

/** A `start_session` (or `send_to_session`) call in the transcript: one
 *  line with the subagent's glyph, title and state, live elapsed while it
 *  runs; it opens the chat's Subagents tab on that subagent. */
export function SubagentLine(props: { name: string; input: unknown; result?: string | null; status?: string }) {
  const call = () => subagentCall(props.name, props.input, props.result);
  const parent = () => chatAgentSessionKey();
  const read = createSubagentsRead(parent);
  const row = () => findRow(read.view(), call()?.sessionKey);
  const now = createSecondClock(() => row()?.status === 'running');
  const asked = () => { const i = props.input as Record<string, unknown> | null; const a = i && typeof i === 'object' ? (i.args && typeof i.args === 'object' ? (i.args as Record<string, unknown>).agent : i.agent) : null; return typeof a === 'string' ? a : ''; };
  const agent = () => row()?.agent ?? asked();
  const label = () => {
    const r = row(), c = call();
    if (r?.title && c) return `${c.kind === 'start' ? 'Started' : 'Messaged'} a subagent: ${r.title}`;
    return c?.label ?? 'Subagent';
  };
  const meta = () => {
    const r = row();
    if (r) return r.status === 'queued' ? statusWords(r) : `${statusWords(r)} · ${durationWords(liveWorkingMs(r, tickFrom(read.view()!.at, now()), now()))}`;
    if (props.status === 'running') return 'Starting…';
    if (props.status === 'error') return 'Failed';
    return '';
  };
  return (
    <button type="button" class="subagent-line" data-status={row()?.status ?? props.status ?? 'done'} disabled={!parent()}
      title="Open it in the Subagents tab" onClick={() => { const key = parent(); if (key) openPreview({ kind: 'subagents', parent: key, ...(call()?.sessionKey ? { open: call()!.sessionKey! } : {}) }); }}>
      <AgentMark agent={agent()} size={14} />
      <span class="subagent-line-text">{label()}</span>
      <Show when={meta()}><span class="subagent-line-meta">{meta()}</span></Show>
    </button>
  );
}
