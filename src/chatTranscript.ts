// The transcript reducer (overnight 10-11 B3): how a turn's streamed text,
// tool calls, results, images and receipt land in a conversation's turns.
// Moved out of store/runtime.ts (the Main agent drawer's engine shared it
// until run 15 P3; store/chatRuntime.ts is its one user now). It owns one
// piece of state, the turn boundary: after `turnBoundary`
// (or a fresh run) the next text starts a new assistant bubble.
import { batch, untrack } from 'solid-js';
import { type SetStoreFunction } from 'solid-js/store';
import type { ApprovalBadge } from './chatApproval';
import type { ExecutionReceipt } from './executionReceipt';
import type { ChatToolUse, ChatTurn } from './chatTurnTypes';

export interface Transcript {
  appendText(text: string): void;
  appendDelta(delta: string): void;
  /** `parentToolUseId`: the spawning call's id for a subagent's call (AG7). */
  appendToolUse(name: string, input?: unknown, id?: string, approval?: ApprovalBadge, parentToolUseId?: string | null): void;
  applyToolResult(id: string, content: string, isError: boolean): void;
  applyToolImages(id: string, images: string[]): void;
  toggleToolExpanded(turnIdx: number, toolIdx: number): void;
  /** Attach a receipt to the newest assistant turn at or after
   *  `turnStartIndex`, or add a bare one (a run that failed before prose). */
  applyReceipt(receipt: ExecutionReceipt, runId: string, turnStartIndex: number): void;
  /** End every partial turn and settle tools still marked running. */
  finalize(): void;
  /** The next text starts a new assistant bubble (`turnBoundary`). */
  markBoundary(): void;
  /** Whether the next text starts a new bubble; snapshots save and restore it. */
  pendingNewTurn(): boolean;
  setPendingNewTurn(value: boolean): void;
}

/** A reducer over one conversation's turns. `runId` names the live run, so a
 *  new assistant bubble carries the run it belongs to; `turns` reads the same
 *  store (see `appendDelta`). */
export function createTranscript(setTurns: SetStoreFunction<ChatTurn[]>, runId: () => string | undefined, turns: () => readonly ChatTurn[]): Transcript {
  let pendingNewTurn = false;
  // Path writes keep Solid's browser store from unwrapping the entire history
  // for every event. Read indexes without creating reactive subscriptions.
  const latest = () => untrack(() => turns().length - 1);
  const findTool = (matches: (tool: ChatToolUse) => boolean) => untrack(() => {
    const arr = turns();
    for (let i = arr.length - 1; i >= 0; i--) {
      for (let j = arr[i].tools.length - 1; j >= 0; j--) {
        if (matches(arr[i].tools[j])) return [i, j] as const;
      }
    }
  });
  const append = (turn: ChatTurn) => {
    setTurns(latest() + 1, turn);
    pendingNewTurn = false;
  };
  return {
    appendText(text) {
      const i = latest();
      if (!pendingNewTurn && untrack(() => turns()[i]?.role === 'assistant')) {
        setTurns(i, 'text', previous => previous + (previous ? '\n\n' : '') + text);
      } else {
        append({ role: 'assistant', text, tools: [], runId: runId() });
      }
    },
    appendDelta(delta) {
      const i = latest();
      if (!pendingNewTurn && untrack(() => turns()[i]?.role === 'assistant' && turns()[i].partial)) {
        setTurns(i, 'text', text => text + delta);
      } else {
        append({ role: 'assistant', text: delta, tools: [], partial: true, runId: runId() });
      }
    },
    appendToolUse(name, input, id, approval, parentToolUseId) {
      const tool: ChatToolUse = approval ? { name, input, id, status: 'running', approval } : { name, input, id, status: 'running' };
      if (parentToolUseId) tool.parentToolUseId = parentToolUseId;
      const i = latest();
      if (!pendingNewTurn && untrack(() => turns()[i]?.role === 'assistant')) {
        setTurns(i, 'tools', untrack(() => turns()[i].tools.length), tool);
      } else {
        append({ role: 'assistant', text: '', tools: [tool], runId: runId() });
      }
    },
    // Newest-first: ids may be reused by historical runs. Without an id
    // (older transcripts) the most recent running tool takes the result.
    applyToolResult(id, content, isError) {
      const match = findTool(tool => id ? tool.id === id : tool.status === 'running');
      if (match) setTurns(match[0], 'tools', match[1], { result: content, isError, status: isError ? 'error' : 'done', endedAt: new Date().toISOString() });
    },
    applyToolImages(id, images) {
      if (!images.length) return;
      const match = findTool(tool => tool.id === id);
      if (match) setTurns(match[0], 'tools', match[1], 'images', images);
    },
    toggleToolExpanded(turnIdx, toolIdx) {
      if (untrack(() => turns()[turnIdx]?.tools[toolIdx])) {
        setTurns(turnIdx, 'tools', toolIdx, 'expanded', expanded => !expanded);
      }
    },
    applyReceipt(receipt, wireRunId, turnStartIndex) {
      untrack(() => batch(() => {
        const arr = turns();
        for (let i = arr.length - 1; i >= turnStartIndex; i--) {
          if (arr[i].role === 'assistant') {
            // Replace this receipt rather than merging stale optional fields.
            setTurns(i, { receipt });
            if (arr[i].runId == null) setTurns(i, 'runId', wireRunId);
            return;
          }
        }
        // A failed run keeps its own receipt, never an earlier request's turn.
        // Receipt-only turns do not consume a pending text boundary.
        setTurns(arr.length, { role: 'assistant', text: '', tools: [], receipt, runId: wireRunId });
      }));
    },
    finalize() {
      untrack(() => batch(() => {
        turns().forEach((turn, i) => {
          if (turn.partial) setTurns(i, 'partial', false);
          turn.tools.forEach((tool, j) => {
            if (tool.status === 'running') setTurns(i, 'tools', j, 'status', 'done');
          });
        });
      }));
    },
    markBoundary() { pendingNewTurn = true; },
    pendingNewTurn: () => pendingNewTurn,
    setPendingNewTurn(value) { pendingNewTurn = value; },
  };
}
