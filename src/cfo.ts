/**
 * The CFO on the frontend (COO rebuild L7): the turn footer's
 * "model · Effort · reason" line and the allowlist editor's model for
 * Settings, Models. Pure functions; the backend
 * is `src-tauri/src/cfo/` (docs/CFO.md).
 *
 * The allowlist is a list of real models per installed agent, taken from
 * the agent's own model list (`agent_models`); there is no free text. An
 * empty list means every installed agent on its saved model.
 */
import type { CfoAllowEntry, CfoDecision } from './generated/ipc';

const EFFORT_WORDS: Record<string, string> = {
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra high',
  max: 'Max',
  ultracode: 'Ultracode',
};

/** "medium" → "Medium"; an unknown level keeps its own word, capitalised. */
export function effortWord(level: string): string {
  const key = level.trim().toLowerCase();
  return EFFORT_WORDS[key] ?? (key ? key[0].toUpperCase() + key.slice(1) : '');
}

/**
 * A short model name from its id, for any provider: a leading vendor word
 * is dropped when a family word follows ("claude-sonnet-5-5" → "Sonnet
 * 5.5"), number parts join with dots, short words read as initials
 * ("gpt-5.5" → "GPT 5.5"), and a context suffix goes ("opus[1m]" → "Opus").
 */
export function modelName(id: string): string {
  const parts = id.replace(/\[[^\]]*\]$/, '').split(/[-_]/).filter(Boolean);
  if (parts.length >= 2 && /^[a-z]+$/i.test(parts[0]) && /^[a-z]+$/i.test(parts[1])) parts.shift();
  const out: string[] = [];
  for (const part of parts) {
    const last = out.length - 1;
    if (/^\d+(\.\d+)*$/.test(part) && last >= 0 && /^\d+(\.\d+)*$/.test(out[last])) {
      out[last] = `${out[last]}.${part}`;
    } else if (/^\d/.test(part)) {
      out.push(part);
    } else {
      out.push(part.length <= 3 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1));
    }
  }
  return out.join(' ') || id;
}

/**
 * The turn footer's CFO line: the model (or the agent, when it ran on its
 * default), the effort with "(you)" when the person chose it, and the
 * reason. `label` names a model from its catalog when known, and `agentLabel`
 * an agent. Null without a decision.
 */
export function cfoFooterLine(
  decision: CfoDecision | null | undefined,
  label?: (model: string) => string | undefined,
  agentLabel?: (agent: string) => string,
): string | null {
  if (!decision) return null;
  const model = decision.model
    ? (label?.(decision.model) ?? modelName(decision.model))
    : (agentLabel?.(decision.agent) ?? (decision.agent ? decision.agent[0].toUpperCase() + decision.agent.slice(1) : ''));
  const effort = decision.effort
    ? `${effortWord(decision.effort)}${decision.effortSource === 'you' ? ' (you)' : ''}`
    : null;
  const parts = [model, effort, decision.reason].filter((part): part is string => !!part && part.trim() !== '');
  return parts.length ? parts.join(' · ') : null;
}

/** One agent's row in the allowlist editor. */
export interface AllowRow {
  agent: string;
  /** "Any model of this agent" is checked. */
  any: boolean;
  /** The models checked, each one of the agent's catalog ids. */
  models: string[];
}

/** The editor's rows for `agents`, from the saved list. */
export function allowRows(allow: readonly CfoAllowEntry[], agents: readonly string[]): AllowRow[] {
  return agents.map(agent => ({
    agent,
    any: allow.some(entry => entry.agent === agent && !entry.model),
    models: allow.filter(entry => entry.agent === agent && entry.model).map(entry => entry.model!),
  }));
}

/** Check or clear "Any model of this agent". Checking it replaces that
 *  agent's single models (any covers them). */
export function setAny(allow: readonly CfoAllowEntry[], agent: string, on: boolean): CfoAllowEntry[] {
  const others = allow.filter(entry => entry.agent !== agent);
  return on ? [...others, { agent }] : others;
}

/** Check or clear one model. Only a model in the agent's `catalog` can be
 *  checked: anything else leaves the list as it was (no free text). */
export function setModel(
  allow: readonly CfoAllowEntry[],
  agent: string,
  model: string,
  on: boolean,
  catalog: readonly string[],
): CfoAllowEntry[] {
  const without = allow.filter(entry => !(entry.agent === agent && entry.model === model));
  if (!on) return without;
  if (!catalog.includes(model)) return [...allow];
  // A single model narrows the agent: "any" for it goes.
  return [...without.filter(entry => !(entry.agent === agent && !entry.model)), { agent, model }];
}

/** Whether the list lets the CFO use this agent and model (empty: all). */
export function isAllowed(allow: readonly CfoAllowEntry[], agent: string, model?: string | null): boolean {
  return allow.length === 0 || allow.some(entry => entry.agent === agent && (!entry.model || entry.model === model));
}

/** The row's one-line summary. */
export function allowSummary(allow: readonly CfoAllowEntry[], agentLabel: (agent: string) => string, modelLabel: (agent: string, model: string) => string): string {
  if (allow.length === 0) return 'Any installed agent, on its saved model. Check models to narrow it.';
  return allow
    .map(entry => entry.model ? `${agentLabel(entry.agent)} ${modelLabel(entry.agent, entry.model)}` : `any ${agentLabel(entry.agent)} model`)
    .join(', ');
}

/** "Room for unattended work", or "Past the reserve until 3:00 PM". */
export function accountRoomLine(room: boolean, roomAt: string | null | undefined, note: string, now = new Date()): string {
  if (room) return note.startsWith('Not measured') ? note : 'Room for unattended work';
  if (!roomAt) return note;
  const at = new Date(roomAt);
  if (Number.isNaN(at.getTime())) return note;
  const sameDay = at.toDateString() === now.toDateString();
  const time = at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  const when = sameDay ? time : `${at.toLocaleDateString([], { weekday: 'short' })} ${time}`;
  return note.startsWith('Out of included usage') ? `${note} (resets ${when})` : `Past the reserve until ${when}`;
}

/**
 * A token amount typed in Settings, Usage, Budgets (L9): plain digits or a
 * k / M suffix ("2M", "500k", "1.5m", "1,200,000"). Blank is no cap (null);
 * anything else, zero included, is 'invalid'.
 */
export function parseTokenAmount(text: string): number | null | 'invalid' {
  const raw = text.trim().replace(/[\s,_]/g, '').toLowerCase();
  if (!raw) return null;
  const match = /^(\d+(?:\.\d+)?)([km]?)$/.exec(raw);
  if (!match) return 'invalid';
  const scale = match[2] === 'm' ? 1_000_000 : match[2] === 'k' ? 1_000 : 1;
  const value = Math.round(Number(match[1]) * scale);
  return Number.isSafeInteger(value) && value > 0 ? value : 'invalid';
}

/** 1200000 → "1.2M", 2000000 → "2M", 500000 → "500k", 950 → "950". */
export function formatTokens(n: number): string {
  const round = (x: number) => Math.round(x * 10) / 10;
  // 999,950 rounds to 1000k: show it as 1M instead (L9 review).
  if (n >= 1_000_000 || round(n / 1_000) >= 1_000) return `${round(n / 1_000_000)}M`;
  if (n >= 1_000) return `${round(n / 1_000)}k`;
  return String(Math.round(n));
}

/** A cap as an input shows it: short when that reads back exactly, else the
 *  whole number, so saving one field never rounds another (L9 review). */
export function tokenInput(n: number | undefined | null): string {
  if (!n) return '';
  const short = formatTokens(n);
  return parseTokenAmount(short) === n ? short : String(n);
}
