/**
 * The next step a finished reply leaves the user, offered as a suggestion in
 * the empty composer. It goes through the normal send, as if typed, so the
 * agent does the work with its usual cards and checks; nothing here merges,
 * installs, deploys or does anything else on its own.
 *
 * Two sources, the agent's own first:
 *  - The agent's guess (2026-10-01). Every chat turn's prompt asks the agent
 *    to end its reply with a fenced `hollow-next` block of one to three
 *    messages the user is likely to send next (src-tauri/src/next_step.rs);
 *    it knows the request, the workspace brief and plan, and what it just
 *    did. The transcript hides the block (src/chatSegments.ts).
 *  - Ship offers, a heuristic over the reply's own words and tool calls,
 *    never the network: a PR the run opened (`gh pr create`) or a reply that
 *    names one (`PR #12`, a GitHub `/pull/12` link) counts as open unless the
 *    same reply merged it (`gh pr merge`, "merged") or set it aside (closed,
 *    superseded). Install vs deploy follows what the reply talks about (inline
 *    code doesn't count); install only in a Hollow build workspace; with
 *    neither, only Merge is offered. They still catch a reply from an agent
 *    that left the block out.
 *
 * Some suggestions carry `review`: the first Enter (or the button, then
 * named Place suggestion) places the text in the box and a second Enter
 * sends it. That is every ship offer, every agent line of a reply that
 * earned one or that names shipping itself (P0-01: one Enter must never
 * merge or install), and every suggestion after a reply that read outside
 * content (a web page, a browser tab, a connector's results; durability D2).
 * Plain Enter still sends the rest (the #248 decision).
 *
 * Pure and dependency-free (vitest in node).
 */

import { splitNextFence } from './chatSegments';

export type ShipActionId = 'merge-install' | 'merge-deploy' | 'merge' | 'install' | 'deploy';

export interface ShipAction {
  /** A ship offer's id, or `next-<n>` for the agent's n-th suggestion. */
  id: ShipActionId | `next-${number}`;
  label: string;
  /** What the offer rests on (next-level P20): the PR state Hollow checked on
   *  GitHub (`checked`), only the reply's words (`reply`), or the agent's
   *  own suggestion (`agent`). */
  basis?: 'checked' | 'reply' | 'agent';
  /** Exactly what the button sends, as if typed. */
  prompt: string;
  /** Enter places the text for the user to check instead of sending it: the
   *  reply read outside content, or the line merges, installs or deploys. */
  review?: boolean;
  /** Why it waits: `outside` content, or a `ship` step (P0-01). */
  reviewWhy?: 'outside' | 'ship';
}

export const SHIP_ACTIONS: Record<ShipActionId, ShipAction> = {
  'merge-install': { id: 'merge-install', label: 'Merge and install', prompt: 'Merge and install' },
  'merge-deploy': { id: 'merge-deploy', label: 'Merge and deploy', prompt: 'Merge and deploy' },
  merge: { id: 'merge', label: 'Merge', prompt: 'Merge it' },
  install: { id: 'install', label: 'Build and install', prompt: 'Build and install it' },
  deploy: { id: 'deploy', label: 'Deploy', prompt: 'Deploy it' },
};

/** Offers that merge first; the others follow a PR already merged. */
const MERGES: ReadonlySet<ShipActionId> = new Set(['merge-install', 'merge-deploy', 'merge']);

/** The fields of a tool call the detector reads (a subset of `ChatToolUse`). */
export interface ShipTool {
  name: string;
  input?: unknown;
  result?: string;
  isError?: boolean;
}

/** The fields of a chat turn the offer reads (a subset of `ChatTurn`). */
export interface ShipTurn {
  role: 'user' | 'assistant' | 'system';
  text: string;
  tools?: readonly ShipTool[];
  partial?: boolean;
  runId?: string;
  hydratedNotice?: boolean;
}

export interface ShipState {
  busy: boolean;
  queued: boolean;
  /** The run id still streaming, if any. */
  liveRunId?: string | null;
}

/** A PR reference: a GitHub pull link, or `PR #12` / `PRs #12` / `pull request #12`. */
const PR_REF = /github\.com\/[\w.-]+\/[\w.-]+\/pull\/(\d+)|\b(?:PRs?|pull requests?)\s+#(\d+)/gi;
/** "merged", but not "not merged", "can be merged", "hasn't been merged"; `\b` already skips "unmerged". */
const MERGED = /(?<!\bnot |\bnot yet |n't |n't been |\bnot been |\bbe |\bto be )\bmerged\b/i;
/** A PR the reply sets aside rather than ships. */
const SET_ASIDE = /\b(?:close[ds]?|closing|superseded|abandon(?:ed)?|drop(?:ped)?)\b/i;
const INSTALL = /\b(?:install(?:ed|ing|s)?|reinstall(?:ed)?|self[- ]install|install_built_app)\b/i;
const DEPLOY = /\b(?:deploy(?:ed|ing|s|ment|ments)?|vercel|netlify|cloud run|fly\.io|heroku|render\.com|railway)\b/i;
/** A reply that merged but says the install or deploy hasn't happened. */
const NOT_INSTALLED = /\b(?:not|isn[’']t|aren[’']t|wasn[’']t|hasn[’']t been|not yet|never) (?:been )?(?:installed|built and installed)\b|\binstalled app is still\b|\b(?:build|built) and install\b[^.?!]*\?/i;
const NOT_DEPLOYED = /\b(?:not|isn[’']t|aren[’']t|wasn[’']t|hasn[’']t been|not yet|never) (?:been )?deployed\b|\bdeploy (?:it|this|now)\b[^.?!]*\?/i;

function commandOf(tool: ShipTool): string {
  const input = tool.input as { command?: unknown; cmd?: unknown } | string | undefined;
  if (typeof input === 'string') return input;
  if (input && typeof input === 'object') {
    const c = input.command ?? input.cmd;
    if (typeof c === 'string') return c;
    if (Array.isArray(c)) return c.join(' ');
  }
  return '';
}

/** Each PR the reply's text mentions: open, merged or set aside. A reference
 *  is judged by its own stretch of the sentence (from the previous reference
 *  to the next), so "merged PR #1; PR #2 is ready" keeps #2 open. */
function textMentions(text: string): Map<number, 'open' | 'merged' | 'aside'> {
  const out = new Map<number, 'open' | 'merged' | 'aside'>();
  // Code blocks quote commands and logs, not what happened.
  const prose = text.replace(/```[\s\S]*?```/g, ' ');
  for (const clause of prose.split(/\n|;|(?<=[.!?])\s+/)) {
    const refs = [...clause.matchAll(PR_REF)];
    refs.forEach((m, i) => {
      const n = Number(m[1] ?? m[2]);
      const from = i === 0 ? 0 : refs[i - 1].index! + refs[i - 1][0].length;
      const to = i + 1 < refs.length ? refs[i + 1].index! : clause.length;
      const window = clause.slice(from, to);
      const state = SET_ASIDE.test(window) ? 'aside' : MERGED.test(window) ? 'merged' : 'open';
      const prev = out.get(n);
      // Merged or set aside anywhere in the reply wins over an open mention.
      if (!prev || prev === 'open') out.set(n, state);
    });
  }
  return out;
}

/** Each PR the reply and its tool calls touch, with what happened to it.
 *  `mergedAll` is a bare `gh pr merge` (the branch's own PR). */
function pullRequests(text: string, tools: readonly ShipTool[]): { state: Map<number, 'open' | 'merged' | 'aside'>; mergedAll: boolean } {
  const state = textMentions(text);
  let mergedAll = false;
  for (const tool of tools) {
    if (tool.isError) continue;
    const cmd = commandOf(tool);
    if (/\bgh\s+pr\s+create\b/.test(cmd)) {
      const url = /\/pull\/(\d+)/.exec(tool.result ?? '');
      if (url && !state.has(Number(url[1]))) state.set(Number(url[1]), 'open');
    }
    const merge = /\bgh\s+pr\s+merge\b(?:\s+(\d+))?/.exec(cmd);
    if (merge) {
      if (merge[1]) state.set(Number(merge[1]), 'merged');
      else mergedAll = true;
    }
  }
  return { state, mergedAll };
}

/** The PRs this reply leaves open, from its tool calls and its text. */
export function openPullRequests(text: string, tools: readonly ShipTool[] = []): number[] {
  const { state, mergedAll } = pullRequests(text, tools);
  if (mergedAll) return [];
  return [...state].filter(([, s]) => s === 'open').map(([n]) => n).sort((a, b) => a - b);
}

/** The PRs this reply merged. */
function mergedPullRequests(text: string, tools: readonly ShipTool[]): { numbers: number[]; any: boolean } {
  const { state, mergedAll } = pullRequests(text, tools);
  const numbers = [...state].filter(([, s]) => s === 'merged').map(([n]) => n);
  return { numbers, any: mergedAll || numbers.length > 0 };
}

/** Which buttons a finished reply earns, most likely first. Inline code
 *  (`npm install`, a `deploy` script) is a command name, not a plan to
 *  install or deploy (P0-01). `hollow`: the install variants are Hollow's own
 *  self-install, offered only in a Hollow build workspace; elsewhere an open
 *  PR gets Merge. */
export function shipActionsFor(text: string, tools: readonly ShipTool[] = [], hollow = false): ShipAction[] {
  const prose = text.replace(/```[\s\S]*?```/g, ' ').replace(/`[^`\n]*`/g, ' ');
  if (openPullRequests(text, tools).length === 0) {
    // Merged in this reply, with the install or deploy still to do.
    if (!mergedPullRequests(text, tools).any) return [];
    const after: ShipAction[] = [];
    if (hollow && NOT_INSTALLED.test(prose)) after.push(SHIP_ACTIONS.install);
    if (NOT_DEPLOYED.test(prose)) after.push(SHIP_ACTIONS.deploy);
    return after;
  }
  const out: ShipAction[] = [];
  if (hollow && INSTALL.test(prose)) out.push(SHIP_ACTIONS['merge-install']);
  if (DEPLOY.test(prose)) out.push(SHIP_ACTIONS['merge-deploy']);
  return out.length ? out : [SHIP_ACTIONS.merge];
}

/** Longest suggestion kept; the prompt asks for under 80 characters. */
const NEXT_MAX = 120;
/** Control and invisible formatting characters (zero-width, bidi overrides):
 *  a line carrying any is dropped, not cleaned. */
const HIDDEN = /[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u2028-\u202e\u2060-\u2064\u2066-\u2069\ufeff]/;

/** The agent's suggested next messages: the lines of the reply's last
 *  `hollow-next` block, at most three, without list markers or quotes. */
export function nextSteps(text: string): ShipAction[] {
  const { blocks } = splitNextFence(text);
  const lines = blocks.at(-1) ?? [];
  const out: ShipAction[] = [];
  const seen = new Set<string>();
  for (const raw of lines) {
    const line = raw.trim()
      .replace(/^(?:[-*•]|\d+[.)])\s+/, '')
      .replace(/^(["“'`])(.*)(["”'`])$/, '$2')
      .trim();
    if (!line || line.length > NEXT_MAX || HIDDEN.test(line) || seen.has(sameStep(line))) continue;
    seen.add(sameStep(line));
    out.push({ id: `next-${out.length}`, label: line, prompt: line, basis: 'agent' });
    if (out.length === 3) break;
  }
  return out;
}

/** What kind of outside content a tool call brought into the turn, if any.
 *  Classes, not one provider's names, so Claude, Codex and the rest match:
 *  - `connector`: any MCP tool (`mcp__<server>__<tool>`, the form every
 *    provider's calls are normalized to) on a server that isn't Hollow's own;
 *  - `browser`: a browser or page tool (the in-app browser, Chrome,
 *    Playwright, Hollow's own `browser_*` tab readers);
 *  - `web`: a web fetch or search tool, or a shell `curl`/`wget` of a URL.
 *  Hollow's own servers (`hollow_tasks`, `hollow_app`, `hollow_vault`,
 *  `hollow_questions`, `hollow_team`, and `hollow_computer` apart from its
 *  browser readers) return Hollow's or the user's own data and don't count. */
export type OutsideClass = 'web' | 'browser' | 'connector';
const BROWSER = /browser|chrome|playwright|puppeteer|bookmark|navigate|screenshot_page|read_page/i;
const WEB = /web[_-]?(?:fetch|search)|^fetch(?:_url)?$|^search_web$|url_context/i;
const SHELL_FETCH = /\b(?:curl|wget)\b[^\n]*\bhttps?:\/\//i;
export function outsideClass(tool: ShipTool): OutsideClass | null {
  const name = tool.name ?? '';
  const mcp = /^mcp__(.+?)__(.+)$/.exec(name);
  if (mcp) {
    const [, server, inner] = mcp;
    if (/^hollow_/i.test(server)) return BROWSER.test(inner) ? 'browser' : null;
    return BROWSER.test(server) || BROWSER.test(inner) ? 'browser' : 'connector';
  }
  if (WEB.test(name)) return 'web';
  if (BROWSER.test(name)) return 'browser';
  if (/^(?:bash|shell|exec|command_execution|run_shell_command)$/i.test(name) && SHELL_FETCH.test(commandOf(tool))) return 'web';
  return null;
}

/** Whether any tool call in the reply's turn read outside content. */
export function readOutsideContent(tools: readonly ShipTool[]): boolean {
  return tools.some(t => outsideClass(t) !== null);
}

/** Two suggestions that say the same thing ("Merge and install it." and
 *  "merge and install") compare equal. */
function sameStep(s: string): string {
  return s.toLowerCase().replace(/[.!\s]+$/, '').replace(/\s+(?:it|this|them)$/, '').trim();
}

/** The offer under the conversation's last reply: only while the chat is
 *  idle, nothing is queued, and the last reply finished with no message
 *  after it. Restored-history notices are skipped; any other notice (a stop,
 *  an error) ends the offer. */
/** The PR state Hollow itself checked for this workspace (rail git state). */
export interface CheckedPr { state: string | null; number: number | null }

/** A suggested line that merges, installs, deploys or otherwise ships: it
 *  waits for a second Enter wherever it came from (P0-01). */
const SHIPS = /\b(?:merg(?:e|es|ed|ing)|(?:re)?install\w*|(?:re)?deploy\w*|ship(?:s|ping)?|land|release|publish)\b/i;

/** `isHollow`: this chat runs in a Hollow build workspace, the only place the
 *  install offers apply. Every ship offer carries `review` (P0-01), and so
 *  does every agent line when the reply earned a ship offer (a "Yes, go
 *  ahead" there is a merge too) or when the line itself ships: one Enter or a
 *  click on Send places it in the box; only a second Enter sends it. */
export function shipOffer(turns: readonly ShipTurn[], state: ShipState, checked?: CheckedPr | null, isHollow = false): ShipAction[] {
  if (state.busy || state.queued) return [];
  let end = turns.length - 1;
  while (end >= 0 && turns[end].role === 'system' && turns[end].hydratedNotice) end--;
  const last = turns[end];
  if (!last || last.role !== 'assistant' || last.partial) return [];
  if (last.runId && state.liveRunId && last.runId === state.liveRunId) return [];
  const parts: string[] = [];
  const tools: ShipTool[] = [];
  for (let i = end; i >= 0 && turns[i].role !== 'user'; i--) {
    if (turns[i].role !== 'assistant') continue;
    if (turns[i].text.trim()) parts.unshift(turns[i].text.trim());
    tools.unshift(...(turns[i].tools ?? []));
  }
  const text = parts.join('\n\n');
  const offers = shipActionsFor(text, tools, isHollow);
  // Structured state wins over prose (next-level P20) when it is about the
  // PR the reply names: one GitHub reports merged or closed gets no merge
  // offer whatever the reply says; one it reports open is offered as
  // checked. A check of some other PR says nothing about this one.
  const named = checked?.number != null && (openPullRequests(text, tools).includes(checked.number) || mergedPullRequests(text, tools).numbers.includes(checked.number));
  const merges = offers.filter(o => MERGES.has(o.id as ShipActionId));
  const shut = named && (checked!.state === 'merged' || checked!.state === 'closed');
  const basis: 'checked' | 'reply' = named && checked!.state === 'open' && merges.length ? 'checked' : 'reply';
  const ship = shut && merges.length ? [] : offers.map(o => ({ ...o, basis }));
  // The agent's own suggestions lead; a ship offer saying the same thing is
  // dropped, and once GitHub reports the PR shut, so is "Merge …".
  const agent = nextSteps(text).filter(o => !(shut && /^merge\b/i.test(o.prompt)));
  const said = new Set(agent.map(o => sameStep(o.prompt)));
  const all = [...agent.map(o => (offers.length || SHIPS.test(o.prompt) ? { ...o, review: true, reviewWhy: 'ship' as const } : o)),
    ...ship.filter(o => !said.has(sameStep(o.prompt))).map(o => ({ ...o, review: true, reviewWhy: 'ship' as const }))];
  return readOutsideContent(tools) ? all.map(o => ({ ...o, review: true, reviewWhy: 'outside' as const })) : all;
}
