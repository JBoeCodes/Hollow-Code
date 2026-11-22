/**
 * One model-catalog request per agent per session, shared by every model
 * picker (the backend also caches each list for a few minutes). `refresh`
 * asks the agent again (the picker's Refresh). A failed load is not cached.
 */
import { api } from './api';
import type { AgentModelCatalog } from './types';

const catalogs = new Map<string, Promise<AgentModelCatalog>>();

export function loadCatalog(agent: string, refresh = false): Promise<AgentModelCatalog> {
  if (refresh || !catalogs.has(agent)) {
    const pending = api.agentModels(agent, refresh).catch(error => {
      // A late rejection must never evict a newer Refresh request.
      if (catalogs.get(agent) === pending) catalogs.delete(agent);
      return { models: [], source: 'Models', warning: String(error), checkedAt: null, newestByFamily: {} };
    });
    catalogs.set(agent, pending);
  }
  return catalogs.get(agent)!;
}

export function forgetCatalog(agent: string) {
  catalogs.delete(agent);
}

/** Self-hosted models Q5, Q9: Claude's catalog ends with each enabled
 *  Anthropic-style self-hosted model and Codex's with each OpenAI-style one,
 *  as `self-hosted:<id>` ("Qwen3.8 27B · self-hosted").
 *  Only a chat turn runs one, so the pickers for defaults, automations and
 *  extraction leave them out ({@link withoutChatOnly}); the chat's model
 *  menu lists them. */
export function chatOnlyModel(id: string): boolean {
  return id.trim().startsWith('self-hosted:');
}

export function withoutChatOnly<T extends { id: string }>(models: readonly T[]): T[] {
  return models.filter(model => !chatOnlyModel(model.id));
}

/** The version numbers in a Claude ID after its family, a trailing date
 *  dropped (`claude-haiku-4-5-20251001` → [4, 5]). */
function versionOf(id: string): number[] {
  return id.split('-').slice(2).filter(p => /^\d+$/.test(p) && p.length < 8).map(Number);
}

function greater(a: number[], b: number[]): boolean {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? 0, y = b[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

/** The newer model of a pinned Claude ID's family, when there is one
 *  (`claude-sonnet-5` → `claude-sonnet-5-5`). Aliases (`sonnet`) already
 *  follow the newest, so they never get one; nor does an ID at or past what
 *  the alias resolves to (a dated snapshot, a preview). */
export function newerModel(value: string, catalog: Pick<AgentModelCatalog, 'newestByFamily'> | null | undefined): string | null {
  const id = value.trim().replace(/\[1m\]$/, '');
  const family = /^claude-([a-z]+)-/.exec(id)?.[1];
  const newest = family ? catalog?.newestByFamily?.[family] : undefined;
  return newest && newest !== id && greater(versionOf(newest), versionOf(id)) ? newest : null;
}

/** "checked 12:40" (today) or "checked Oct 12" for a catalog's time. */
export function checkedLabel(at: string | null | undefined, now = new Date()): string {
  if (!at) return '';
  const when = new Date(at);
  if (Number.isNaN(when.getTime())) return '';
  const today = when.toDateString() === now.toDateString();
  return `checked ${today
    ? when.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
    : when.toLocaleDateString([], { month: 'short', day: 'numeric' })}`;
}
