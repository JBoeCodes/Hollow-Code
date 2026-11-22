// The chat's notice for an agent that can't take Hollow's per-run MCP config
// (merged plan slice 2 item 4). Grok 1.0.46 and agy 1.2.15 have no such flag
// (their `mcp add` edits only their own global config), so a chat with either
// has no Hollow confirm cards, no question tool and no app tools; Hollow turns
// a fenced `hollow-questions` block at the end of a finished reply into a
// question card instead. Grok also loads the MCP servers in its own ~/.grok
// config, which Hollow can't switch off. The backend is the source:
// `agent::capabilities(..).owned_mcp` (descriptor `ownedMcp`), so a provider
// that gains a per-run config loses the notice there. A teammate's result
// carries the Rust copy (`team_mcp::no_cards_note`). Pure, so vitest can drive it.
import type { AgentDescriptor } from './types';

type Capabilities = AgentDescriptor['capabilities'] | undefined;

/** Agents known to lack Hollow's per-run MCP config, used only until the
 *  descriptors load (mirrors `agent::capabilities`). */
const NO_OWNED_MCP = new Set(['antigravity', 'grok']);

/** The notice for a chat with `agent`, or null when it takes Hollow's tools. */
export function hollowCardsNotice(agent: string, label: string, caps: Capabilities): string | null {
  const owned = caps ? caps.ownedMcp : !NO_OWNED_MCP.has(agent);
  if (owned) return null;
  // The alternative, where it's needed (plan completion L4, X13): the agents
  // that do take it are one model-menu switch away.
  const notice = `No Hollow cards or app tools in this chat: ${label} can't take Hollow's per-run tool setup. Nothing stops mid-run to ask you; a question can come only as a card at the end of a reply. For Hollow's cards and tools, switch this chat to Claude or Codex in the model menu.`;
  return agent === 'grok' ? `${notice} MCP servers set up in ~/.grok still load.` : notice;
}
