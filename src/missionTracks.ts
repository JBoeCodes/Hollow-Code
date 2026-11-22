// Pure projection for the lazy Mission Control tracks. Queue/feed evidence only.
import type { Feed, MissionCard } from './missionControl';
export interface MissionTrack { id: string; title: string; cards: MissionCard[]; live: number }
export function missionTracks(live: readonly MissionCard[], finished: readonly MissionCard[]): MissionTrack[] {
  const groups = new Map<string, MissionTrack>();
  const seen = new Set<string>();
  for (const c of [...live, ...finished]) {
    if (seen.has(c.id)) continue;
    seen.add(c.id);
    const g = groups.get(c.system) ?? { id: c.system, title: c.systemTitle, cards: [], live: 0 };
    g.cards.push(c);
    if (['running', 'starting', 'queued'].includes(c.status)) g.live++;
    groups.set(c.system, g);
  }
  return [...groups.values()].sort((a, b) => Number(b.live > 0) - Number(a.live > 0) || a.title.localeCompare(b.title) || a.id.localeCompare(b.id))
    .map(g => ({ ...g, cards: g.cards.sort((a, b) => (a.startedAt ?? Infinity) - (b.startedAt ?? Infinity) || a.id.localeCompare(b.id)) }));
}
const WORDS: Record<MissionCard['status'], string> = { running: 'Running', starting: 'Preparing', queued: 'Queued', done: 'Finished', failed: 'Failed', stopped: 'Stopped', skipped: 'Skipped', interrupted: 'Interrupted', paused: 'Paused for requested work' };
/** Labels describe work in progress, never delivery. Only direct shell commands
 * (not arbitrary assistant prose or a matching substring) qualify. */
export function trackPhase(c: MissionCard, feed: Feed): string {
  if (c.status !== 'running') return WORDS[c.status];
  const f = c.key ? feed[c.key] : null;
  if (!f?.running || f.queueId !== c.queueId || !f.tool) return 'Running';
  if (!/^(Bash|bash|shell|exec_command)$/.test(f.tool.name)) return 'Running';
  const command = (f.tool.input.command ?? f.tool.input.cmd ?? '').trim();
  if (/^(?:npm run gate(?::(?:frontend|rust))?|npm test|npx vitest run|cargo (?:test|clippy))(?=\s|$)/.test(command)) return 'Gating';
  if (/^(?:npm run build(?::mock)?|cargo build)(?=\s|$)/.test(command)) return 'Building';
  return 'Running';
}
/** Cost coverage of these returned turns, not a night total or budget. */
export function trackCost(cards: readonly MissionCard[]): { knownUsd: number | null; known: number; unknown: number; pending: number; total: number } {
  const unique = [...new Map(cards.map(c => [c.id, c])).values()];
  let known = 0, sum = 0, pending = 0;
  for (const c of unique) {
    if (['running', 'starting', 'queued'].includes(c.status)) { pending++; continue; }
    if (typeof c.costUsd !== 'number' || !Number.isFinite(c.costUsd) || c.costUsd < 0) continue;
    known++; sum += c.costUsd;
  }
  return { knownUsd: known ? sum : null, known, unknown: unique.length - known - pending, pending, total: unique.length };
}
