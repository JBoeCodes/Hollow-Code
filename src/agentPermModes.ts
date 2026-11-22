// Each agent's own permission modes (per-agent permission menus, 2026-11-12).
// Hollow keeps five canonical modes (plan, default, acceptEdits, auto,
// bypassPermissions: `app_mcp::PERMISSION_MODES`), and the backend passes
// each one straight to the chat's CLI (`agent.rs`: Claude and Grok
// `--permission-mode`, Codex's sandbox and approval policy, Antigravity's
// `--mode`/`--sandbox` or `--dangerously-skip-permissions`). This table is
// how a chat names them: the agent's own mode names and order, a short chip
// name, one honest line on what a Hollow run of that agent really does, and
// which mode skips every check (danger). Pure data and helpers, no store, so
// vitest drives it and the shell chunk only gains this small module.

export type CanonicalMode = 'plan' | 'default' | 'acceptEdits' | 'auto' | 'bypassPermissions';
export const CANONICAL_MODES: readonly CanonicalMode[] = ['plan', 'default', 'acceptEdits', 'auto', 'bypassPermissions'];
export type PermAgent = 'claude' | 'codex' | 'grok' | 'antigravity';
export const PERM_AGENTS: readonly PermAgent[] = ['claude', 'codex', 'grok', 'antigravity'];

export interface AgentPermMode {
  /** The canonical mode Hollow stores and sends. */
  value: CanonicalMode;
  /** The agent's own name for it. */
  label: string;
  /** The composer's access chip. */
  chip: string;
  /** One line: what a Hollow run of this agent does in this mode. */
  description: string;
  /** Skips every check: the pickers ask first and the chip warns. */
  danger: boolean;
}

const mode = (value: CanonicalMode, label: string, description: string, chip = label, danger = false): AgentPermMode =>
  ({ value, label, chip, description, danger });

/** Every agent's options in its own order. Hollow runs every agent without a
 *  terminal, so a mode that would stop to ask can't: each line says what
 *  happens instead (`agent.rs` `build_claude`, `codex_app_server_policy`,
 *  `grok_sandbox`, `build_antigravity`; docs/reference/agent-tools.md
 *  "Permission modes across agents"). */
export const AGENT_PERM_MODES: Record<PermAgent, readonly AgentPermMode[]> = {
  // Claude Code's own cycle (Default, Accept edits, Plan), then Auto and Bypass.
  claude: [
    mode('default', 'Default', 'Claude Code’s standard mode: it asks before edits and commands. Hollow runs it with no terminal to ask in, so those are declined and it reads and answers.'),
    mode('acceptEdits', 'Accept edits', 'File edits in its folders go ahead without asking. Commands it would ask about are declined.'),
    mode('plan', 'Plan', 'Claude reads and plans, and changes nothing.'),
    mode('auto', 'Auto', 'Claude’s own classifier approves each action and blocks risky ones.'),
    mode('bypassPermissions', 'Bypass permissions', 'Every tool runs with no prompt and no check.', 'Bypass', true),
  ],
  // Codex's approval presets, safest first. Hollow runs `codex exec` (or its
  // app server) with approvals off except in Approve for me, which also gets
  // the chat's folders and linked worktree git dirs as writable roots and the
  // network (`agent.rs` `codex_auto_writable_roots`).
  codex: [
    mode('plan', 'Read only', 'Codex reads files and answers in a read-only sandbox. It can’t edit or run anything that writes.'),
    mode('default', 'Ask for approval', 'Codex edits and runs commands inside the working folder’s sandbox. Hollow runs it with no terminal, so it never stops to ask: anything beyond the sandbox, like other folders or the network, fails instead.'),
    mode('auto', 'Approve for me', 'Codex edits and runs commands in the working folder and the chat’s worktree, can commit and use the network, and its automatic reviewer decides each request beyond that.'),
    mode('bypassPermissions', 'Full access', 'No sandbox and no approvals: Codex can run anything on this Mac.', 'Full access', true),
  ],
  // Grok's `--permission-mode` names. Its headless runs act without asking
  // in every mode, so the sandbox profile (`grok_sandbox`) is the real limit.
  // `dontAsk` exists in grok's CLI too; Hollow has no canonical mode for it.
  grok: [
    mode('default', 'Default', 'Grok can’t ask while Hollow runs it, so it runs in the read-only sandbox: it reads and answers, and changes nothing outside its own folders.'),
    mode('acceptEdits', 'Accept edits', 'Edits and commands run in the workspace sandbox: only the working folder and temporary folders can change.'),
    mode('plan', 'Plan', 'Grok plans in the read-only sandbox and changes nothing.'),
    mode('auto', 'Auto', 'Grok approves its own actions with the sandbox off, so it can change anything on this Mac.', 'Auto', true),
    mode('bypassPermissions', 'Bypass permissions', 'No prompts and no sandbox: anything runs.', 'Bypass', true),
  ],
  // agy's `--mode` values, then `--dangerously-skip-permissions`. Over ACP
  // (a chat) each request agy would ask about is an approval card.
  antigravity: [
    mode('acceptEdits', 'Accept edits', 'Antigravity edits files in the working folder and asks before a command: each request is an Allow card (a run without a chat, such as an automation, can’t ask and refuses commands).'),
    mode('plan', 'Plan', 'Antigravity plans in its sandbox and changes nothing.'),
    mode('auto', 'Auto', 'Antigravity’s own reviewer: edits in the working folder and safe commands go ahead, writes outside the folder are refused, and the rest ask.'),
    mode('bypassPermissions', 'Skip permissions', 'Every tool runs with no prompt and no sandbox.', 'Skip permissions', true),
  ],
};

/** A canonical mode an agent has no option of its own for, and the option
 *  that runs the same way there: Codex's `acceptEdits` and `default` both
 *  run the workspace-write sandbox with approvals off; Antigravity runs
 *  `default` as `--mode accept-edits`. */
const SAME_AS: Record<PermAgent, Partial<Record<CanonicalMode, CanonicalMode>>> = {
  claude: {},
  codex: { acceptEdits: 'default' },
  grok: {},
  antigravity: { default: 'acceptEdits' },
};

/** Narrower modes to fall back on, in order, when a mode can't run. */
const NARROWER: Record<CanonicalMode, readonly CanonicalMode[]> = {
  bypassPermissions: ['auto', 'acceptEdits', 'default', 'plan'],
  auto: ['acceptEdits', 'default', 'plan'],
  acceptEdits: ['default', 'plan'],
  default: ['plan'],
  plan: [],
};

/** The agent's table; an unknown agent reads as Claude's (Hollow's default agent). */
export function agentPermModes(agent: string | null | undefined): readonly AgentPermMode[] {
  return AGENT_PERM_MODES[(agent ?? '') as PermAgent] ?? AGENT_PERM_MODES.claude;
}

const sameAs = (agent: string | null | undefined, m: string): string =>
  (SAME_AS[(agent ?? '') as PermAgent] ?? {})[m as CanonicalMode] ?? m;

/** The option a chat's stored mode shows as for `agent`: its own, or the one
 *  that runs the same way (a Codex chat stored as acceptEdits shows Ask for
 *  approval). Null for a value no option covers (an unknown string). */
export function agentPermOption(agent: string | null | undefined, m: string): AgentPermMode | null {
  const shown = sameAs(agent, m);
  return agentPermModes(agent).find(o => o.value === shown) ?? null;
}

/** The option value a stored mode shows as (`agentPermOption`), else itself. */
export function shownMode<M extends string>(agent: string | null | undefined, m: M): M {
  return (agentPermOption(agent, m)?.value ?? m) as M;
}

/** The agent's options in its order, as canonical values. */
export function agentModeOrder(agent: string | null | undefined): CanonicalMode[] {
  return agentPermModes(agent).map(o => o.value);
}

function byMode(agent: string | null | undefined, pick: (o: AgentPermMode) => string): Record<CanonicalMode, string> {
  return Object.fromEntries(CANONICAL_MODES.map(m => {
    const o = agentPermOption(agent, m) ?? agentPermOption(agent, nearestMode(agent, m));
    return [m, o ? pick(o) : m];
  })) as Record<CanonicalMode, string>;
}

/** Every canonical mode's name for `agent` (a mode it has no option for
 *  reads as the option it shows as). */
export function agentModeLabels(agent: string | null | undefined): Record<CanonicalMode, string> { return byMode(agent, o => o.label); }
/** The access chip's names for `agent`. */
export function agentModeChips(agent: string | null | undefined): Record<CanonicalMode, string> { return byMode(agent, o => o.chip); }
/** Every canonical mode's one-line description for `agent`. */
export function agentModeDescriptions(agent: string | null | undefined): Record<CanonicalMode, string> { return byMode(agent, o => o.description); }
/** Whether a stored mode skips every check for `agent`. */
export function agentModeDanger(agent: string | null | undefined): Record<CanonicalMode, boolean> {
  return Object.fromEntries(CANONICAL_MODES.map(m => [m, !!agentPermOption(agent, m)?.danger])) as Record<CanonicalMode, boolean>;
}

/** The mode a chat moving to `agent` (or starting there) runs: `m` when the
 *  agent has an option for it that can run, the option that runs the same
 *  way (Codex: Accept edits is Ask for approval), else the nearest narrower
 *  one that can (Antigravity has no Auto: Accept edits). `unavailable` is
 *  the agent's descriptor `unavailableModes` (mode to reason). Never wider. */
export function nearestMode(agent: string | null | undefined, m: string, unavailable?: Record<string, string> | null): CanonicalMode {
  const own = new Set(agentModeOrder(agent));
  const runs = (c: CanonicalMode) => own.has(c) && !(unavailable?.[c] ?? '').trim();
  const start = (CANONICAL_MODES as readonly string[]).includes(m) ? m as CanonicalMode : 'acceptEdits';
  for (const c of [start, ...NARROWER[start]]) {
    const shown = sameAs(agent, c) as CanonicalMode;
    if (runs(shown)) return shown;
  }
  return 'plan';
}

const agentName = (agent: string) => ({ claude: 'Claude', codex: 'Codex', grok: 'Grok', antigravity: 'Antigravity' } as Record<string, string>)[agent] ?? agent;

/** A chat switching from `from` to `to` in mode `m`: the mode it runs on
 *  `to` and the one line that says so, or '' when the mode carries over
 *  under the same name. */
export function agentSwitchMode(from: string, to: string, m: string, unavailable?: Record<string, string> | null): { mode: CanonicalMode; note: string } {
  const before = agentPermOption(from, m)?.label ?? m;
  let next = nearestMode(to, m, unavailable);
  // A switch never lands in a mode that skips every check unless the chat
  // already was in one: Grok's Auto runs with its sandbox off, so a Claude
  // Auto chat moving there takes the nearest narrower mode instead (picking
  // it is one confirm away in the menu).
  if (agentModeDanger(to)[next] && !agentPermOption(from, m)?.danger) {
    const safer = NARROWER[next].map(c => nearestMode(to, c, unavailable)).find(c => !agentModeDanger(to)[c]) ?? 'plan';
    const after = agentModeLabels(to)[safer];
    return { mode: safer, note: `Permissions: ${agentModeLabels(to)[next]} on ${agentName(to)} skips every check, so this chat now uses ${after}.` };
  }
  const after = agentModeLabels(to)[next];
  if (next === m && before === after) return { mode: next, note: '' };
  const own = agentPermOption(to, m);
  const note = own && own.value === next
    ? `Permissions: ${before} on ${agentName(from)} is ${after} on ${agentName(to)}.`
    : own
      ? `Permissions: ${agentName(to)} can’t run ${own.label}, so this chat now uses ${after}.`
      : `Permissions: ${agentName(to)} has no ${before}, so this chat now uses ${after}.`;
  return { mode: next, note };
}

/** The menus' shared last line: Hollow's own cards for its actions sit on top
 *  of whatever mode the agent runs in, for the agents that take Hollow's
 *  tools (`app_actions::asks_in_mode`; Grok and Antigravity can't). */
export function hollowCardsLine(agent: string | null | undefined, ownedMcp?: boolean): string {
  const a = agent ?? 'claude';
  const owned = ownedMcp ?? (a === 'claude' || a === 'codex');
  if (!owned) return `Hollow’s own cards can’t reach ${agentName(a)}, so these modes are its only limit.`;
  const bypass = agentPermModes(a).find(o => o.value === 'bypassPermissions')?.label ?? 'Bypass';
  return `Hollow still asks on a card before its own deletes, sends, secrets and installs in every mode; ${bypass} skips all but the few that always ask.`;
}

/** The mode a new chat with `agent` starts in when nothing was saved: Auto
 *  for Claude and Codex, Accept edits for Grok and Antigravity (Antigravity
 *  has no Auto). */
export const AGENT_NEW_CHAT_DEFAULT: Record<PermAgent, CanonicalMode> = {
  claude: 'auto',
  codex: 'auto',
  grok: 'acceptEdits',
  antigravity: 'acceptEdits',
};

/** Where an agent's own new-chat mode is saved (Settings, Chat). */
export const newChatModeKey = (agent: string) => `new${agent.charAt(0).toUpperCase()}${agent.slice(1)}PermissionMode`;

/** The mode a new chat with `agent` starts in: the agent's own saved
 *  default, else the agent's default, mapped to an option the agent can run
 *  (`nearestMode`). The single setting from before per-agent defaults is
 *  retired (owner decision, 2026-11-12): it was seeded from the last chat's
 *  mode, never chosen. */
export function newChatModeFor(agent: string, own?: string | null, unavailable?: Record<string, string> | null): CanonicalMode {
  const known = (v: string | null | undefined): v is CanonicalMode => !!v && (CANONICAL_MODES as readonly string[]).includes(v);
  const pick = known(own) ? own : AGENT_NEW_CHAT_DEFAULT[agent as PermAgent] ?? 'acceptEdits';
  return nearestMode(agent, pick, unavailable);
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z]+/g, '-').replace(/^-|-$/g, '');

/** The words `/permissions` offers for `agent`: its own mode names. */
export function permissionsCommandWords(agent: string | null | undefined): string[] {
  return agentPermModes(agent).map(o => slug(o.label));
}

/** The canonical mode `/permissions <words>` names for `agent`: one of its
 *  own names (any case, spaces or dashes), a canonical value, or the short
 *  words from before (bypass, edits); null when none matches. */
export function permissionsCommandMode(agent: string | null | undefined, words: string): CanonicalMode | null {
  const w = slug(words);
  if (!w) return null;
  const own = agentPermModes(agent).find(o => slug(o.label) === w || slug(o.chip) === w);
  if (own) return own.value;
  const canonical = CANONICAL_MODES.find(m => m.toLowerCase() === w.replace(/-/g, ''));
  if (canonical) return canonical;
  const before: Record<string, CanonicalMode> = { bypass: 'bypassPermissions', edits: 'acceptEdits', 'edits-only': 'acceptEdits', 'full-access': 'bypassPermissions', ask: 'default' };
  return before[w] ?? null;
}
