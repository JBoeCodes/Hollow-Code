// The composer's slash commands. Extracted from ChatBody.tsx (plan L3 X17,
// 2026-10-14; review V06/F11): Hollow's own commands (LOCAL_COMMANDS, which
// src/inventoryMap.test.ts reads by name), Claude Code's discovered ones, and
// running a local command or `claude mcp|plugin`. Moved unchanged, no
// behaviour change.
//
// Created once, synchronously, in ChatBody's body. Discovery is shared by
// every composer (one module-level promise); whether this composer asked for
// it is its own.
import { createMemo, createSignal } from 'solid-js';
import {
  chatCwd,
  chatBusy,
  setChatBusy,
  chatModel, setChatModelPersistent,
  chatAgent, switchChatAgent, AGENT_LABELS, currentAgentCapabilities, agentsAvailable,
  chatPermissionMode, setChatPermissionModePersistent,
  confirmPermissionModeChange, sameChatAsNow,
  resetChatState, applyNewChatPermissionMode, pushChatTurn, setChatCwdPersistent, setChatInput,
  stopChatRun, pauseChatRun, canPauseChatRun, chatPausedHere, resumePausedChat,
  flashFooter,
  type ChatPermissionMode,
  type ChatAgent,
} from '../../store';
import { api } from '../../api';
import { permModeUnavailable } from '../../permModes';
import { agentModeDescriptions, agentModeLabels, nearestMode, permissionsCommandMode, permissionsCommandWords, shownMode } from '../../agentPermModes';
import { unsafeCwdNotice, unsafeCwdText } from '../../chatCwd';
import { workspaceDetail, newWorkspaceSessionOpened } from '../../store/workspaces';
import { isMainAgent } from '../../workspaceTypes';

/** A local command while a turn runs (P1-30): 'refuse' one that changes the
 *  next turn or needs the composer idle (its text kept), 'run' a read or
 *  /stop, null for anything that isn't a local command (it steers or queues
 *  as words). /mcp and /plugin hold the busy flag while they run, so they
 *  wait too (review). */
export function busyLocalCommand(prompt: string): 'refuse' | 'run' | null {
  const [head, ...rest] = prompt.trim().split(/\s+/);
  const name = (head ?? '').toLowerCase();
  if (!prompt.startsWith('/')) return null;
  if (['/new', '/clear', '/mcp', '/plugin', '/plugins'].includes(name)) return 'refuse';
  if (['/model', '/agent', '/cwd', '/permissions'].includes(name)) return rest.length ? 'refuse' : 'run';
  return name === '/help' || name === '/stop' || name === '/pause' ? 'run' : null;
}

/** Why a local command waits for the running turn (P1-30). Nothing runs it
 *  later: the person sends it again. */
export function busyCommandRefusal(prompt: string): string {
  const name = prompt.trim().split(/\s+/)[0].toLowerCase();
  const what = name === '/new' || name === '/clear' ? 'starts a new chat'
    : name === '/mcp' || name.startsWith('/plugin') ? "can't run during a turn" : 'changes the next turn';
  return `${name} ${what}: send it again once this turn ends, or Stop it first. It's still in the box.`;
}

export interface SlashCmd {
  name: string;
  desc: string;
  source: 'local' | 'claude';
  hasArgs?: boolean;
  /** An alias that still runs when typed but isn't listed (P1-27: /clear). */
  hidden?: true;
}

const LOCAL_COMMANDS: SlashCmd[] = [
  { name: '/new',    source: 'local', desc: 'Start a new chat (clears history and session)' },
  { name: '/clear',  source: 'local', desc: 'Alias for /new', hidden: true },
  { name: '/cwd',    source: 'local', desc: 'Show or set the working directory', hasArgs: true },
  { name: '/agent',  source: 'local', desc: 'Switch agent (claude | codex | antigravity | grok)', hasArgs: true },
  { name: '/model',  source: 'local', desc: 'Switch the active agent’s model', hasArgs: true },
  { name: '/mcp',    source: 'local', desc: 'List Claude Code MCP integrations', hasArgs: true },
  { name: '/plugin', source: 'local', desc: 'List Claude Code plugins',  hasArgs: true },
  { name: '/permissions', source: 'local', desc: 'Show or set permission mode (bypass | auto | edits | plan)', hasArgs: true },
  { name: '/stop',   source: 'local', desc: 'Cancel the currently running turn' },
  { name: '/pause',  source: 'local', desc: 'Pause the running turn at its next safe point (the composer’s Pause)' },
  { name: '/resume', source: 'local', desc: 'Resume a paused chat (the composer’s Resume)' },
  { name: '/help',   source: 'local', desc: 'List local slash commands' },
];

let _discoverPromise: Promise<string[]> | null = null;
function discoverOnce(): Promise<string[]> {
  if (!_discoverPromise) _discoverPromise = api.chatDiscover().catch(() => [] as string[]);
  return _discoverPromise;
}

export interface SlashCommandDeps {
  /** A command's output goes to the end of the transcript, like a send. */
  followSend: () => void;
}

export function createSlashCommands(deps: SlashCommandDeps) {
  const { followSend } = deps;
  const [claudeCommands, setClaudeCommands] = createSignal<SlashCmd[]>([]);
  const mainChat = () => isMainAgent(workspaceDetail()?.workspace) && workspaceDetail()?.workspace.cwd === chatCwd();

  const allCommands = createMemo<SlashCmd[]>(() => {
    const localNames = new Set(LOCAL_COMMANDS.map(c => c.name));
    const claude = chatAgent() === 'claude' ? claudeCommands().filter(c => !localNames.has(c.name)) : [];
    claude.sort((a, b) => a.name.localeCompare(b.name));
    return [...visibleLocalCommands(), ...claude];
  });

  function visibleLocalCommands() {
    return LOCAL_COMMANDS.filter(c => !c.hidden && (chatAgent() === 'claude' || !['/mcp', '/plugin'].includes(c.name)));
  }

  let _discoverStarted = false;
  function ensureDiscovered() {
    // Slash-command discovery probes Claude's system/init event; codex/agy
    // don't advertise commands that way, so only discover for Claude.
    if (_discoverStarted || chatAgent() !== 'claude' || !agentsAvailable().includes('claude')) return;
    _discoverStarted = true;
    discoverOnce().then(names => {
      setClaudeCommands(names.map(n => ({ name: '/' + n, source: 'claude', desc: '', hasArgs: false })));
    });
  }

  function handleLocalCommand(prompt: string): boolean {
    if (!prompt.startsWith('/')) return false;
    const space = prompt.indexOf(' ');
    const name = (space === -1 ? prompt : prompt.slice(0, space)).toLowerCase();
    const args = space === -1 ? '' : prompt.slice(space + 1).trim();

    switch (name) {
      case '/new':
      case '/clear': {
        if (chatBusy()) return true;
        // The command itself is consumed, never saved as an unsent draft.
        setChatInput('');
        if (mainChat()) {
          void newWorkspaceSessionOpened(workspaceDetail()!.workspace.id);
          return true;
        }
        resetChatState();
        applyNewChatPermissionMode();
        return true;
      }
      case '/cwd': {
        if (!args) {
          pushChatTurn({ role: 'system', text: chatCwd() ? `Working directory: ${chatCwd()}` : 'Working directory: vault (default)', tools: [] });
        } else if (args === '-' || args === 'reset' || args === 'default') {
          setChatCwdPersistent(null);
          pushChatTurn({ role: 'system', text: 'Working directory reset to vault (default)', tools: [] });
        } else {
          // Paperclip PR 19: a system or shared folder (`/`, `/tmp`, bare `~`,
          // …) is refused before it becomes the conversation's folder.
          const requested = args;
          api.chatPreflight(requested).then(() => {
            setChatCwdPersistent(requested);
            pushChatTurn({ role: 'system', text: `Working directory set to ${requested}`, tools: [] });
          }).catch(err => {
            const unsafe = unsafeCwdText(err);
            const text = unsafe ? unsafeCwdNotice(unsafe) : `(error: could not verify ${requested}: ${String(err)})`;
            pushChatTurn({ role: 'system', text, tools: [] });
            flashFooter(unsafe ? '× working folder refused' : '× could not verify the folder', 4000);
          });
        }
        return true;
      }
      case '/agent': {
        const aliases: Record<string, ChatAgent> = {
          claude: 'claude',
          codex: 'codex',
          antigravity: 'antigravity', agy: 'antigravity',
          grok: 'grok',
        };
        if (!args) {
          pushChatTurn({ role: 'system', text: `Agent: **${AGENT_LABELS[chatAgent()]}** (${chatAgent()})\n\nSwitch with: \`/agent claude | codex | antigravity | grok\``, tools: [] });
        } else {
          const next = aliases[args.toLowerCase()];
          if (next && !agentsAvailable().includes(next)) {
            pushChatTurn({ role: 'system', text: `${AGENT_LABELS[next]} is not available. Install and sign in to it, then refresh Models in Settings.`, tools: [] });
          } else if (next) {
            const moved = switchChatAgent(next);
            pushChatTurn({ role: 'system', text: `Agent → **${AGENT_LABELS[next]}** (model reset to ${chatModel() || '(default)'})${moved ? `\n${moved}` : ''}`, tools: [] });
          } else {
            pushChatTurn({ role: 'system', text: `Unknown agent: ${args}. Use claude, codex, antigravity, or grok.`, tools: [] });
          }
        }
        return true;
      }
      case '/model': {
        if (!args) {
          pushChatTurn({ role: 'system', text: `Model: ${chatModel() || '(agent default)'} — ${AGENT_LABELS[chatAgent()]}. Use /model followed by a supported model ID, or /model default.`, tools: [] });
        } else {
          // Pass the name through verbatim; 'default' clears to the CLI default.
          const norm = (args === 'default' || args === '(default)') ? '' : args;
          setChatModelPersistent(norm);
          pushChatTurn({ role: 'system', text: `Model set to ${norm || '(default)'}`, tools: [] });
        }
        return true;
      }
      case '/permissions': {
        // The chat's agent's own modes (per-agent menus, 2026-11-12): its
        // names, any canonical value, and the short words from before.
        const agent = chatAgent();
        const labels = agentModeLabels(agent);
        if (!args) {
          const cur = shownMode(agent, chatPermissionMode());
          pushChatTurn({
            role: 'system',
            text: `Permission mode: **${labels[cur]}** (${cur})\n${agentModeDescriptions(agent)[cur]}\n\nSet with: \`/permissions ${permissionsCommandWords(agent).join(' | ')}\``,
            tools: [],
          });
        } else {
          const named = permissionsCommandMode(agent, args);
          const norm = named ? nearestMode(agent, named) as ChatPermissionMode : undefined;
          const unavailable = norm ? permModeUnavailable(currentAgentCapabilities(), norm) : null;
          if (unavailable) {
            pushChatTurn({ role: 'system', text: unavailable, tools: [] });
          } else if (norm) {
            // The answer belongs to the chat it was asked on: another chat
            // on screen by then hears nothing (lane S re-review of #1304).
            const still = sameChatAsNow();
            confirmPermissionModeChange(norm, agent).then(ok => {
              if (!still()) return;
              if (!ok) {
                pushChatTurn({ role: 'system', text: `Permission mode change cancelled.`, tools: [] });
                return;
              }
              // A refusal says why instead of claiming the change (P0-19).
              const why = setChatPermissionModePersistent(norm);
              pushChatTurn({ role: 'system', text: why ? `Permission mode not changed: ${why}.` : `Permission mode → **${labels[norm]}** (${norm})`, tools: [] });
            });
          } else {
            pushChatTurn({ role: 'system', text: `Unknown mode: ${args}. Use ${permissionsCommandWords(agent).join(' | ')}.`, tools: [] });
          }
        }
        return true;
      }
      case '/stop':
        // ChatBody's `stop`: a queued turn comes back, a running one stops.
        stopChatRun();
        return true;
      // P1a PR 2: faster routes to the composer's Pause and Resume buttons.
      case '/pause':
        if (canPauseChatRun()) void pauseChatRun();
        else flashFooter('Nothing to pause: no turn of this chat is running.', 4000);
        return true;
      case '/resume':
        if (chatBusy()) flashFooter('This chat is already running.', 4000);
        else if (chatPausedHere()) void resumePausedChat();
        else flashFooter('This chat isn’t paused. Continue or a new message picks a stopped chat up.', 5000);
        return true;
      case '/mcp':
      case '/plugin':
      case '/plugins':
        if (chatAgent() === 'claude') runClaudeAdmin(name, args);
        else pushChatTurn({ role: 'system', text: `${name} manages Claude Code's local integration only. Configure ${AGENT_LABELS[chatAgent()]} in its own CLI; Hollow-owned tools are controlled in Settings.`, tools: [] });
        return true;
      case '/help': {
        const lines = visibleLocalCommands().map(c => `${c.name} — ${c.desc}`);
        lines.push('/clear does the same as /new.', '');
        lines.push('Anything else starting with / is sent to the active agent (it dispatches its own commands).');
        pushChatTurn({ role: 'system', text: lines.join('\n'), tools: [] });
        return true;
      }
      default:
        return false;
    }
  }

  function runClaudeAdmin(cmd: string, argsStr: string) {
    const sub = cmd === '/plugins' ? 'plugin' : cmd.slice(1);
    const parts = argsStr.trim().length === 0 ? [] : argsStr.trim().split(/\s+/);
    if (parts.length === 0) parts.push('list');
    const fullArgs = [sub, ...parts];
    pushChatTurn({ role: 'system', text: `$ claude ${fullArgs.join(' ')}`, tools: [] });
    setChatBusy(true);
    api.claudeAdmin(fullArgs, chatCwd() ?? undefined).then(out => {
      pushChatTurn({ role: 'system', text: '```\n' + out.trim() + '\n```', tools: [] });
    }).catch(err => {
      const msg = err instanceof Error ? err.message : String(err);
      pushChatTurn({ role: 'system', text: `(error: ${msg})`, tools: [] });
    }).finally(() => {
      setChatBusy(false);
      followSend();
    });
  }

  return { claudeCommands, allCommands, ensureDiscovered, handleLocalCommand, runClaudeAdmin };
}

export type SlashCommands = ReturnType<typeof createSlashCommands>;
