/**
 * Where a task came from, in words, and where that leads (consolidation G,
 * 2026-10-13): a task made from a workspace plan, a recording, an agent run
 * or an Incoming item says so in its editor, with a link back, so a task,
 * the plan item it came from and a promise are told apart. Reads the task's
 * stored `origin:` (src-tauri vault_tasks::task_origin). Pure.
 */
export type OriginLink =
  /** The workspace's plan; `item` is the plan item's text, to mark it. */
  | { kind: 'plan'; workspaceId: string; item?: string }
  | { kind: 'workspace'; workspaceId: string }
  /** One recording, by its id. */
  | { kind: 'recording'; id: string }
  /** A note in the vault (a meeting note), by its path. */
  | { kind: 'note'; path: string }
  /** One agent run in History & recovery. */
  | { kind: 'run'; runId: string };

export interface TaskOrigin { text: string; link?: OriginLink; linkLabel?: string; /** How the task and its source relate after creation. */ relation?: string }

/** What the caller knows beyond the origin string: the task's own title
 *  (a plan item is found by the text it was copied from) and the promise a
 *  `commitment:` origin names, when the commitments list is loaded. */
export interface OriginContext {
  taskTitle?: string;
  promiseOf?: (vaultPath: string) => { counterparty: string; direction: 'owed_by_me' | 'owed_to_me' } | null;
}

const WORKSPACE_PLAN = /^plan:workspaces\/([^/]+)\/PLAN\.md$/;

/**
 * Exact where the identifier allows (plan completion item 4): a recording,
 * a promise and an agent run each open as that one item; a plan origin opens
 * the plan with the item the task was copied from marked. Incoming items are
 * consumed when filed, so that origin can only open Incoming itself.
 */
export function describeTaskOrigin(origin: string | null | undefined, titleOf: (workspaceId: string) => string | null, ctx: OriginContext = {}): TaskOrigin | null {
  const o = (origin ?? '').trim();
  if (!o) return null;
  const plan = WORKSPACE_PLAN.exec(o);
  if (plan) {
    const title = titleOf(plan[1]);
    const item = ctx.taskTitle?.trim();
    return {
      text: `An item in ${title ? `“${title}”` : 'a workspace'}’s plan`,
      link: item ? { kind: 'plan', workspaceId: plan[1], item } : { kind: 'plan', workspaceId: plan[1] },
      linkLabel: item ? 'Open the plan item' : 'Open the plan',
      relation: 'Copied from the plan when the task was made; they aren’t kept in sync. Ticking one doesn’t finish the other.',
    };
  }
  if (o.startsWith('plan:')) return { text: `A plan item (${o.slice(5)})`, relation: 'Copied from the plan; not kept in sync.' };
  if (o.startsWith('workspace:')) {
    const id = o.slice(10), title = titleOf(id);
    return { text: `Made in ${title ? `“${title}”` : 'a workspace'}`, link: { kind: 'workspace', workspaceId: id }, linkLabel: 'Open the workspace' };
  }
  if (o.startsWith('meeting:') || o.startsWith('recording:')) {
    const id = o.slice(o.indexOf(':') + 1).split('\n')[0].trim();
    // `meeting:<path>` names a meeting note (vault_tasks' documented form);
    // `meeting:<recording id>` is what Recordings' Create tasks writes.
    if (o.startsWith('meeting:') && (id.includes('/') || /\.md$/i.test(id))) return { text: 'An action item from a meeting note', link: { kind: 'note', path: id }, linkLabel: 'Open the meeting note' };
    return id
      ? { text: 'An action item from a recording', link: { kind: 'recording', id }, linkLabel: 'Open the recording' }
      : { text: 'An action item from a recording' };
  }
  if (o.startsWith('commitment:')) {
    const path = o.slice(11).trim();
    const promise = path ? ctx.promiseOf?.(path) ?? null : null;
    const who = promise ? (promise.direction === 'owed_by_me' ? ` to ${promise.counterparty}` : ` with ${promise.counterparty}`) : '';
    return {
      text: promise ? `A promise you made${who}` : 'A promise you made',
      relation: 'The promise stays in the ledger until it is marked done there; finishing this task does not close it.',
    };
  }
  if (o.startsWith('triage:')) return { text: 'Filed from an incoming item', relation: 'That item was handled when it was filed, so it cannot be reopened from here.' };
  if (o.startsWith('run:')) {
    const runId = o.slice(4).trim();
    return { text: `Proposed by an agent run you approved (${runId.slice(0, 20)})`, ...(runId ? { link: { kind: 'run' as const, runId }, linkLabel: 'Open the run in History' } : {}) };
  }
  if (o.startsWith('companion:')) return { text: 'Added from a companion device' };
  if (o === 'agent') return { text: 'Proposed by an agent; you approved it' };
  return { text: o };
}

/** The `origin:` a task made from a promise carries (the ledger's Make
 *  a task), so the promise can find its task and the task its promise. */
export const commitmentTaskOrigin = (vaultPath: string) => `commitment:${vaultPath}`.slice(0, 200);

/** A plan item as tasks are keyed: whitespace collapsed, lowercase (the
 *  backend's normalize_plan_item, so a match here is the same item). */
export const normalizePlanText = (text: string) => text.split(/\s+/).filter(Boolean).join(' ').toLowerCase();

/** Where a plan-made task's item stands in the plan now, by its copied text:
 *  still open, ticked, or not in the plan as written (renamed or removed;
 *  the task is separate from it either way). */
export type PlanItemState = 'open' | 'done' | 'missing';
export function planItemState(title: string, items: readonly { text: string; done: boolean }[]): PlanItemState {
  const want = normalizePlanText(title);
  const hit = items.find(i => normalizePlanText(i.text) === want);
  return !hit ? 'missing' : hit.done ? 'done' : 'open';
}
export const PLAN_ITEM_STATE_LABEL: Record<PlanItemState, string> = {
  open: 'plan item still open',
  done: 'plan item ticked',
  missing: 'not in the plan as written',
};
