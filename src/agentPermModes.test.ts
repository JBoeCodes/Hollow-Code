import { describe, expect, it } from 'vitest';
import {
  AGENT_NEW_CHAT_DEFAULT, AGENT_PERM_MODES, CANONICAL_MODES, PERM_AGENTS, agentModeChips, agentModeDanger, agentModeDescriptions,
  agentModeLabels, agentModeOrder, agentPermOption, agentSwitchMode, hollowCardsLine, nearestMode, newChatModeFor, newChatModeKey,
  permissionsCommandMode, permissionsCommandWords, shownMode,
} from './agentPermModes';
import { permModeOptions } from './permModes';
import agentRs from '../src-tauri/src/agent.rs?raw';

const ANTIGRAVITY_NO_AUTO = { auto: 'No Auto for Antigravity.' };
// No agent has an unavailable mode since Antigravity's Auto came back (2026-11-20);
// the fixture above still exercises the mechanism.
const unavailableFor = (_agent: string) => ({});

describe('each agent’s own modes (per-agent menus, 2026-11-12)', () => {
  it('names every agent’s modes as that agent does, in its order', () => {
    expect(Object.fromEntries(PERM_AGENTS.map(a => [a, AGENT_PERM_MODES[a].map(o => `${o.label}=${o.value}`)]))).toEqual({
      claude: ['Default=default', 'Accept edits=acceptEdits', 'Plan=plan', 'Auto=auto', 'Bypass permissions=bypassPermissions'],
      codex: ['Read only=plan', 'Ask for approval=default', 'Approve for me=auto', 'Full access=bypassPermissions'],
      grok: ['Default=default', 'Accept edits=acceptEdits', 'Plan=plan', 'Auto=auto', 'Bypass permissions=bypassPermissions'],
      antigravity: ['Accept edits=acceptEdits', 'Plan=plan', 'Auto=auto', 'Skip permissions=bypassPermissions'],
    });
  });

  it.each(PERM_AGENTS)('%s: every canonical mode shows as exactly one option, or is unavailable', agent => {
    for (const mode of CANONICAL_MODES) {
      const option = agentPermOption(agent, mode);
      expect(option, `${agent} ${mode}`).not.toBeNull();
      expect(AGENT_PERM_MODES[agent].filter(o => o === option)).toHaveLength(1);
      // The options a picker builds: the one shown is either runnable or marked unavailable.
      const listed = permModeOptions(agentModeOrder(agent), agentModeLabels(agent), agentModeDescriptions(agent), { unavailableModes: unavailableFor(agent) } as never)
        .filter(o => o.value === shownMode(agent, mode));
      expect(listed).toHaveLength(1);
      expect(listed[0].disabled).toBeUndefined();
    }
  });

  it.each(PERM_AGENTS)('%s: each option’s value round-trips to itself, with no value listed twice', agent => {
    const values = agentModeOrder(agent);
    expect(new Set(values).size).toBe(values.length);
    for (const value of values) {
      expect(shownMode(agent, value)).toBe(value);
      expect(agentPermOption(agent, value)!.value).toBe(value);
      expect(nearestMode(agent, value)).toBe(value);
      // `/permissions` takes the option's own name back to its value.
      expect(permissionsCommandMode(agent, agentPermOption(agent, value)!.label)).toBe(value);
    }
  });

  it('shows a mode an agent has no option for as the one that runs the same way', () => {
    // Codex runs acceptEdits and default alike (workspace-write, approvals off).
    expect(agentModeLabels('codex').acceptEdits).toBe('Ask for approval');
    expect(shownMode('codex', 'acceptEdits')).toBe('default');
    // Antigravity runs default as `--mode accept-edits`.
    expect(agentModeLabels('antigravity').default).toBe('Accept edits');
    expect(shownMode('antigravity', 'default')).toBe('acceptEdits');
    expect(agentModeChips('claude').bypassPermissions).toBe('Bypass');
  });

  it('flags only the modes that skip every check as danger', () => {
    const flagged = Object.fromEntries(PERM_AGENTS.map(a => [a, CANONICAL_MODES.filter(m => agentModeDanger(a)[m])]));
    // Grok's Auto runs with its sandbox off (`grok_sandbox`), so it is one too.
    expect(flagged).toEqual({ claude: ['bypassPermissions'], codex: ['bypassPermissions'], grok: ['auto', 'bypassPermissions'], antigravity: ['bypassPermissions'] });
  });

  it('says honestly what a run without a terminal does', () => {
    expect(agentModeDescriptions('codex').default).toMatch(/never stops to ask/);
    expect(agentModeDescriptions('codex').auto).toMatch(/chat’s worktree, can commit and use the network/);
    expect(agentModeDescriptions('grok').default).toMatch(/read-only sandbox/);
    expect(agentModeDescriptions('antigravity').acceptEdits).toMatch(/edits files in the working folder and asks before a command/);
    expect(agentModeDescriptions('claude').default).toMatch(/declined/);
  });

  it('matches what the backend passes each CLI', () => {
    // Grok's sandbox per mode, and Antigravity running default as accept-edits.
    expect(agentRs).toContain('"plan" | "default" => "read-only",');
    expect(agentRs).toContain('"auto" | "bypassPermissions" => "off",');
    expect(agentRs).toMatch(/cmd\.arg\("--mode"\)\.arg\(match spec\.perm\.trim\(\) \{\s*"plan" => "plan",[\s\S]*?"auto" => "auto",\s*_ => "accept-edits"/);
    // Codex Auto: the chat's folders writable and the network on (part B).
    expect(agentRs).toContain('"sandbox_workspace_write.network_access=true"');
    // Codex: read-only for plan only, a writable workspace for the rest.
    expect(agentRs).toMatch(/fn codex_sandbox\(perm: &str\) -> &'static str \{\s*if perm == "plan" \{\s*"read-only"\s*\} else \{\s*"workspace-write"/);
  });

  it('has no dash characters in anything it shows', () => {
    const shown = PERM_AGENTS.flatMap(a => AGENT_PERM_MODES[a].flatMap(o => [o.label, o.chip, o.description]));
    shown.push(...PERM_AGENTS.map(a => hollowCardsLine(a)));
    for (const text of shown) expect(text).not.toMatch(/[\u2013\u2014]/);
  });
});

describe('the nearest mode on another agent', () => {
  it.each([
    // [from, mode, to, mode there, line]
    ['claude', 'acceptEdits', 'codex', 'default', 'Permissions: Accept edits on Claude is Ask for approval on Codex.'],
    ['claude', 'plan', 'codex', 'plan', 'Permissions: Plan on Claude is Read only on Codex.'],
    ['claude', 'auto', 'codex', 'auto', 'Permissions: Auto on Claude is Approve for me on Codex.'],
    ['claude', 'bypassPermissions', 'codex', 'bypassPermissions', 'Permissions: Bypass permissions on Claude is Full access on Codex.'],
    ['claude', 'default', 'codex', 'default', 'Permissions: Default on Claude is Ask for approval on Codex.'],
    ['claude', 'auto', 'antigravity', 'auto', ''],
    ['claude', 'default', 'antigravity', 'acceptEdits', 'Permissions: Default on Claude is Accept edits on Antigravity.'],
    ['claude', 'bypassPermissions', 'antigravity', 'bypassPermissions', 'Permissions: Bypass permissions on Claude is Skip permissions on Antigravity.'],
    ['codex', 'default', 'claude', 'default', 'Permissions: Ask for approval on Codex is Default on Claude.'],
    ['codex', 'acceptEdits', 'claude', 'acceptEdits', 'Permissions: Ask for approval on Codex is Accept edits on Claude.'],
    ['antigravity', 'acceptEdits', 'grok', 'acceptEdits', ''],
    ['claude', 'plan', 'grok', 'plan', ''],
    ['grok', 'auto', 'claude', 'auto', ''],
    // Never into a mode that skips every check unless the chat already was in one.
    ['claude', 'auto', 'grok', 'acceptEdits', 'Permissions: Auto on Grok skips every check, so this chat now uses Accept edits.'],
    ['codex', 'auto', 'grok', 'acceptEdits', 'Permissions: Auto on Grok skips every check, so this chat now uses Accept edits.'],
    ['claude', 'bypassPermissions', 'grok', 'bypassPermissions', ''],
  ])('%s %s to %s: %s', (from, mode, to, expected, line) => {
    expect(agentSwitchMode(from, to, mode, unavailableFor(to))).toEqual({ mode: expected, note: line });
  });

  it('never widens: an unavailable mode falls to a narrower one, down to Plan', () => {
    expect(nearestMode('antigravity', 'auto', ANTIGRAVITY_NO_AUTO)).toBe('acceptEdits');
    expect(nearestMode('claude', 'bypassPermissions', { bypassPermissions: 'no', auto: 'no', acceptEdits: 'no', default: 'no' })).toBe('plan');
    expect(nearestMode('codex', 'nonsense')).toBe('default');
    // A blank reason doesn't count as unavailable.
    expect(nearestMode('antigravity', 'auto', { auto: '  ' })).toBe('auto');
  });
});

describe('new chats', () => {
  it('start Claude and Codex in Auto, Grok and Antigravity in Accept edits, saved per agent', () => {
    expect(AGENT_NEW_CHAT_DEFAULT).toEqual({ claude: 'auto', codex: 'auto', grok: 'acceptEdits', antigravity: 'acceptEdits' });
    expect(PERM_AGENTS.map(newChatModeKey)).toEqual(['newClaudePermissionMode', 'newCodexPermissionMode', 'newGrokPermissionMode', 'newAntigravityPermissionMode']);
  });

  it('take the agent’s own saved mode, else the agent’s default; the single setting from before is retired', () => {
    expect(newChatModeFor('claude')).toBe('auto');
    expect(newChatModeFor('codex', null)).toBe('auto');
    expect(newChatModeFor('claude', 'plan')).toBe('plan');
    expect(newChatModeFor('codex', 'acceptEdits')).toBe('default');
    expect(newChatModeFor('antigravity', 'auto', ANTIGRAVITY_NO_AUTO)).toBe('acceptEdits');
    expect(newChatModeFor('grok', 'junk')).toBe('acceptEdits');
  });
});

describe('/permissions words', () => {
  it('offers each agent’s own names and takes the old short words too', () => {
    expect(permissionsCommandWords('codex')).toEqual(['read-only', 'ask-for-approval', 'approve-for-me', 'full-access']);
    expect(permissionsCommandMode('claude', 'Accept edits')).toBe('acceptEdits');
    expect(permissionsCommandMode('claude', 'bypass')).toBe('bypassPermissions');
    expect(permissionsCommandMode('codex', 'edits')).toBe('acceptEdits');
    expect(permissionsCommandMode('codex', 'acceptEdits')).toBe('acceptEdits');
    expect(permissionsCommandMode('antigravity', 'skip permissions')).toBe('bypassPermissions');
    expect(permissionsCommandMode('grok', 'nope')).toBeNull();
  });
});

describe('the shared line about Hollow’s own cards', () => {
  it('names the agent’s own bypass, or says the cards can’t reach it', () => {
    expect(hollowCardsLine('codex')).toBe('Hollow still asks on a card before its own deletes, sends, secrets and installs in every mode; Full access skips all but the few that always ask.');
    expect(hollowCardsLine('grok')).toBe('Hollow’s own cards can’t reach Grok, so these modes are its only limit.');
    expect(hollowCardsLine('antigravity', false)).toMatch(/can’t reach Antigravity/);
  });
});
