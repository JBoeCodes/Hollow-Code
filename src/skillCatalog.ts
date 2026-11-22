/**
 * One Skills catalog (audit H13). `list_skills` returns every installation,
 * including the same name from several sources (skills.rs never hides a
 * duplicate). The catalog lists each name once, explains who owns each copy
 * and who can use it, and orders duplicates by the backend's precedence:
 * Hollow's shared library, then provider installs, then plugins, then
 * marketplace. Pure, so it is unit-tested in node.
 *
 * How a skill reaches an agent (agent effectiveness F9, wave 4 · V1; the
 * contract is `capability_contract.rs`, rendered in docs/AGENT_CONTEXT.md):
 * Hollow attaches no skill to any run. A chat turn lists the skills from
 * the vault's .agents/skills and ~/.agents/skills that fit its request in
 * its <hollow-skills> block (wave 4 · V2, `skill_index.rs`), which the agent
 * reads whole on demand. Use in workspace asks the agent to read the chosen
 * SKILL.md, through Hollow's app tools (`skills.read_skill`) when it has
 * them, else with its own file tools; the turn then lists that skill first,
 * with its text when small. A provider's own runtime
 * may also load the skills installed for it, which Hollow doesn't verify per
 * run; Codex plugins are off in Hollow's Codex runs, so their skills don't
 * load there. The copy here says exactly that and no more.
 */
import type { Skill, SkillLint } from './types';

export type SkillOwner = {
  /** Short badge: "Shared", "Claude", "Codex", "Codex system", "Plugin". */
  badge: string;
  /** Filter bucket. */
  kind: 'shared' | 'provider' | 'plugin' | 'marketplace';
  /** One sentence: who uses this copy and whether Hollow can edit it. */
  usedBy: string;
  /** Precedence rank (0 first), matching skills.rs list_all_in's sort. */
  rank: number;
};

const PROVIDERS: Record<string, string> = { claude: 'Claude', codex: 'Codex', 'codex-system': 'Codex system' };

export function skillOwner(s: Pick<Skill, 'source' | 'sourceKind' | 'plugin' | 'writable'>): SkillOwner {
  if (s.source === 'shared') return {
    badge: 'Shared', kind: 'shared', rank: 0,
    usedBy: `Hollow’s shared library. No agent loads it by itself; a chat lists it to the agent when a request fits, and its Use in button asks the agent to read it.${s.writable ? ' Editable here.' : ''}`,
  };
  if (s.source.startsWith('provider:')) {
    const id = s.source.slice('provider:'.length);
    const name = PROVIDERS[id] ?? id;
    const runtime = id.startsWith('codex') ? 'Codex' : name;
    return {
      badge: name, kind: 'provider', rank: 1,
      usedBy: `${id === 'codex-system' ? 'Bundled with Codex' : `Installed for ${runtime}`}. ${runtime}’s own runtime may load it by itself (Hollow doesn’t verify that per run); its Use in button asks any agent to read it. Read-only here.`,
    };
  }
  if (s.sourceKind === 'plugin') {
    // "plugin:<provider>/<market>/<plugin>@<version>"
    const m = /^plugin:([^/]+)\/([^/]+)\/([^@]+)@?(.*)$/.exec(s.source);
    const runtime = m ? (PROVIDERS[m[1]] ?? m[1]) : 'its agent';
    // Hollow's Codex runs disable plugins (`--disable plugins`).
    const off = m?.[1] === 'codex' ? ' Plugins are off in Hollow’s Codex runs, so Codex doesn’t load it there.' : '';
    return {
      badge: 'Plugin', kind: 'plugin', rank: 2,
      usedBy: `From the ${m?.[3] ?? s.plugin ?? 'installed'} plugin for ${runtime}${m?.[2] ? ` (${m[2]}${m[4] ? `, ${m[4]}` : ''})` : ''}.${off} Read-only here.`,
    };
  }
  return { badge: 'Marketplace', kind: 'marketplace', rank: 3, usedBy: `From ${s.plugin ?? 'a marketplace plugin'}. Read-only here.` };
}

export type CatalogEntry = {
  name: string;
  /** Every copy of this name, in precedence order; [0] is listed first. */
  copies: Skill[];
  /** Best description available (first non-empty in precedence order). */
  description: string;
};

/** Group by name (case-sensitive, as agents invoke them) in precedence order,
 *  entries sorted by name. */
export function buildCatalog(skills: readonly Skill[]): CatalogEntry[] {
  const byName = new Map<string, Skill[]>();
  for (const s of skills) (byName.get(s.name) ?? byName.set(s.name, []).get(s.name)!).push(s);
  return [...byName.entries()].map(([name, copies]) => {
    const ordered = [...copies].sort((a, b) => skillOwner(a).rank - skillOwner(b).rank || a.source.localeCompare(b.source) || a.path.localeCompare(b.path));
    return { name, copies: ordered, description: ordered.find(c => c.description.trim())?.description ?? '' };
  }).sort((a, b) => a.name.localeCompare(b.name));
}

export type CatalogFilter = 'all' | SkillOwner['kind'];

/** Entries with at least one copy in the filter bucket that match every
 *  search term in name, description or owner badge. */
export function filterCatalog(entries: readonly CatalogEntry[], query: string, filter: CatalogFilter): CatalogEntry[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  return entries.filter(e => {
    if (filter !== 'all' && !e.copies.some(c => skillOwner(c).kind === filter)) return false;
    const hay = `${e.name} ${e.description} ${e.copies.map(c => skillOwner(c).badge).join(' ')}`.toLowerCase();
    return terms.every(t => hay.includes(t));
  });
}

/** Plain explanation of what happens with several copies of one name. */
export function precedenceNote(entry: CatalogEntry): string | null {
  if (entry.copies.length < 2) return null;
  const order = entry.copies.map(c => skillOwner(c).badge).join(' → ');
  return `${entry.copies.length} copies of “${entry.name}”, listed in precedence order: ${order}. The Use in button asks the agent to read the copy you choose; a provider’s own runtime may also load its own copy. Nothing is hidden or merged.`;
}

/** Counts per filter bucket (entries, not copies), for the filter labels. */
export function catalogCounts(entries: readonly CatalogEntry[]): Record<CatalogFilter, number> {
  const out: Record<CatalogFilter, number> = { all: entries.length, shared: 0, provider: 0, plugin: 0, marketplace: 0 };
  for (const e of entries) for (const k of new Set(e.copies.map(c => skillOwner(c).kind))) out[k]++;
  return out;
}

// ── Availability for one agent (next-level P18) ────────────────────────────

/** The runtime that may load a copy by itself, or null (shared,
 *  marketplace, and Codex plugins, which Hollow's Codex runs switch off):
 *  matches the owners above. */
export function skillRuntime(s: Pick<Skill, 'source'>): string | null {
  if (s.source.startsWith('provider:')) return s.source.slice('provider:'.length).startsWith('codex') ? 'codex' : s.source.slice('provider:'.length);
  const m = /^plugin:([^/]+)\//.exec(s.source);
  return m && m[1] !== 'codex' ? m[1] : null;
}

export interface SkillAvailability {
  /** The agent's own runtime may load a copy without being asked (Hollow
   *  doesn't verify that per run). */
  auto: boolean;
  /** The copy that applies for this agent: the one its runtime may load,
   *  else the first in precedence order (what Use in workspace offers first). */
  copy: Skill;
  /** One sentence for the detail pane. */
  text: string;
}

/** How `agent` gets this skill in a conversation. Hollow loads no skill into
 *  a run: Use in workspace asks the agent to read the chosen copy; only the
 *  runtime that installed a copy may also load it on its own. */
export function skillAvailability(entry: CatalogEntry, agent: string, agentLabel: string): SkillAvailability {
  const own = entry.copies.find(c => skillRuntime(c) === agent);
  if (own) {
    const others = entry.copies.length > 1 ? ` The other ${entry.copies.length === 2 ? 'copy is' : 'copies are'} read only if you choose ${entry.copies.length === 2 ? 'it' : 'one'} with the Use in button.` : '';
    return { auto: true, copy: own, text: `${agentLabel}’s own runtime may load the ${skillOwner(own).badge} copy by itself; Hollow doesn’t verify that per run, so the Use in button is the sure way.${others}` };
  }
  const first = entry.copies[0];
  const listed = skillOwner(first).kind === 'shared' ? ' A chat also lists it to the agent when a request fits.' : '';
  return { auto: false, copy: first, text: `${agentLabel} doesn’t load it by itself. The Use in button asks it to read the ${skillOwner(first).badge} copy for one request.${listed}` };
}

/** What Use in workspace puts in the composer for `skill`: a request to read
 *  the SKILL.md first, through Hollow's app tools when the agent has them
 *  (they read any listed skill, also from a working folder whose mode
 *  refuses files outside it), else with its own file tools. Provider-
 *  neutral: every agent gets the same words. */
export function useSkillPrompt(skill: Pick<Skill, 'name' | 'path'>): string {
  const path = JSON.stringify(skill.path);
  return `Use the skill ${JSON.stringify(skill.name)} for this task. First read its instructions: with Hollow’s app tools, run_action skills.read_skill with {"path": ${path}}; without them, open ${path} with your own file tools (if that file is outside what you may read, say so and ask me to paste it). Then follow them for this task:\n\n`;
}

// ── Skill lint badge (NV1) ─────────────────────────────────────────────────

/** The lint of an entry's shared copy (the only copy that carries one), or null. */
export function entryLint(entry: CatalogEntry): SkillLint | null {
  return entry.copies.find(c => c.lint)?.lint ?? null;
}

export interface LintBadge {
  /** "Lint 85", or "Lint 100" for a clean skill. */
  label: string;
  grade: SkillLint['grade'];
  /** Hover text: the grade and how many findings there are. */
  title: string;
}

/** The badge a skill row shows: the score, colored by grade. A clean skill
 *  still shows it, so a missing badge always means "not checked" (a provider
 *  or plugin copy), never "fine". */
export function lintBadge(lint: SkillLint): LintBadge {
  const n = lint.findings.length;
  const words = n === 0 ? 'no findings' : `${n} ${n === 1 ? 'finding' : 'findings'}`;
  return { label: `Lint ${lint.score}`, grade: lint.grade, title: `Skill lint: ${lint.grade}, ${words}. Open the skill for the fixes.` };
}

/** Findings worst first (largest deduction, then area order), for the detail list. */
export function lintFindingsInOrder(lint: SkillLint): SkillLint['findings'] {
  const area = ['safety', 'schema', 'description', 'size'];
  return [...lint.findings].sort((a, b) => b.deduction - a.deduction || area.indexOf(a.area) - area.indexOf(b.area) || a.rule.localeCompare(b.rule));
}
