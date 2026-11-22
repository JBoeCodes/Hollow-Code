/**
 * Pure helpers for the composer's model menu (ChatModelMenu): one searchable
 * list of every installed agent's models, grouped by agent. Type-only
 * imports, so the node vitest suite can load it.
 */

export interface MenuModel {
  id: string;
  label: string;
  /** Effort levels this model takes, lowest first, when narrower than the agent's (Antigravity's base models). */
  levels?: string[];
}

/** The catalog row a saved model id means: itself, else (Antigravity) the
 *  base model a saved level variant (`gemini-4-neon-high`) belongs to, since
 *  the picker lists bases and Hollow's one Effort control sets the level. */
export function catalogModel(models: readonly MenuModel[] | undefined, id: string): MenuModel | undefined {
  if (!id) return undefined;
  const exact = models?.find(m => m.id === id);
  if (exact) return exact;
  return models?.find(m => (m.levels ?? []).some(level => id === `${m.id}-${level}`));
}

/** The levels the Effort row offers: the agent's, narrowed to what the chat's
 *  model takes when its catalog row says (Neon has Extra high, Argon stops at
 *  High). Never empty when the agent offers levels. */
export function levelsForModel(agentLevels: readonly string[], models: readonly MenuModel[] | undefined, id: string): string[] {
  const own = catalogModel(models, id)?.levels ?? [];
  if (!own.length) return [...agentLevels];
  const narrowed = agentLevels.filter(level => own.includes(level));
  return narrowed.length ? narrowed : [...agentLevels];
}

export interface MenuRow {
  agent: string;
  /** '' is the agent's default model. */
  id: string;
  label: string;
  /** A model the catalog does not list (the current one, or a typed ID). */
  custom?: boolean;
  /** A "Use “x” with <agent>" row built from the search text. */
  typed?: boolean;
}

export interface MenuGroup { agent: string; agentLabel: string; rows: MenuRow[] }

function matches(query: string, ...fields: string[]): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return q.split(/\s+/).every(term => fields.some(f => f.toLowerCase().includes(term)));
}

/** Groups in agent order. Each lists its default, its catalog models and, when
 *  it is the current agent and the current model is not in its catalog, that
 *  model too, so the checked row never disappears. A search keeps only rows
 *  whose model or agent name matches every word. */
export function menuGroups(
  agents: { id: string; label: string }[],
  catalogs: Record<string, MenuModel[] | undefined>,
  current: { agent: string; model: string },
  query: string,
): MenuGroup[] {
  return agents
    .map(agent => {
      const models = catalogs[agent.id] ?? [];
      const rows: MenuRow[] = [{ agent: agent.id, id: '', label: `${agent.label} default` }];
      for (const m of models) rows.push({ agent: agent.id, id: m.id, label: m.label || m.id });
      if (agent.id === current.agent && current.model && !catalogModel(models, current.model)) {
        rows.push({ agent: agent.id, id: current.model, label: current.model, custom: true });
      }
      return {
        agent: agent.id,
        agentLabel: agent.label,
        rows: rows.filter(r => matches(query, r.label, r.id, agent.label, agent.id)),
      };
    })
    .filter(group => group.rows.length > 0);
}

/** "Use “x” with <agent>" rows for a typed ID that no catalog lists exactly
 *  (an exact match is already a row, so offering it elsewhere is noise). */
export function customRows(
  agents: { id: string; label: string }[],
  catalogs: Record<string, MenuModel[] | undefined>,
  query: string,
): MenuRow[] {
  const id = query.trim();
  if (!id || /\s/.test(id) || id.length > 120) return [];
  if (agents.some(agent => (catalogs[agent.id] ?? []).some(m => m.id === id))) return [];
  return agents
    .map(agent => ({ agent: agent.id, id, label: `Use “${id}” with ${agent.label}`, custom: true, typed: true }));
}

/** The trigger's model text: the catalog label, else the raw ID, else the
 *  agent's default. */
export function modelLabel(agentLabel: string, model: string, models: MenuModel[] | undefined): string {
  if (!model) return `${agentLabel} default`;
  return catalogModel(models, model)?.label || model;
}

/** A model's family for Retry: a context suffix dropped (`opus[1m]` → `opus`)
 *  and a pinned ID that a family's alias already resolves to named by that
 *  alias (`claude-opus-5-5` → `opus`, from the catalog's `newestByFamily`). */
function retryFamily(id: string, newest: Record<string, string>): string {
  const base = id.replace(/\[[^\]]*\]$/, '');
  for (const [family, pinned] of Object.entries(newest)) if (pinned === base) return family;
  return base;
}

/** The models a reply can be retried with (overnight 11-07, D1, audit gap 10):
 *  one row per family of the current agent's catalog (the 1M-context twin and
 *  the pinned ID of a family already listed are the same model), minus the
 *  family the chat uses now, at most `limit` of them (More lists each as
 *  "Retry with <model>"). With a `newestByFamily` (Claude), the families come
 *  first in catalog order, then self-hosted models, then the rest (a mixed
 *  mode such as Opus plans, an older pinned model), so the four rows are the
 *  four families. The agent's default (an empty model) excludes nothing:
 *  Hollow doesn't know which model the CLI's own default is. */
export function retryModelChoices(models: readonly MenuModel[] | undefined, current: string, options: { limit?: number; newestByFamily?: Record<string, string> } = {}): MenuModel[] {
  const newest = options.newestByFamily ?? {};
  const families = new Set(Object.keys(newest));
  const seen = new Set<string>();
  if (current.trim()) seen.add(retryFamily(current.trim(), newest));
  const rows: { model: MenuModel; rank: number }[] = [];
  for (const m of models ?? []) {
    if (!m.id) continue;
    const family = retryFamily(m.id, newest);
    if (seen.has(family)) continue;
    seen.add(family);
    const rank = !families.size || families.has(family) ? 0 : m.id.startsWith('self-hosted:') ? 1 : 2;
    rows.push({ model: { id: m.id, label: m.label || m.id }, rank });
  }
  // A stable sort keeps the catalog's order inside each rank.
  return rows.sort((a, b) => a.rank - b.rank).slice(0, options.limit ?? 4).map(row => row.model);
}

/** "high" → "High"; undefined when the agent has no effort set. */
export function effortLabel(level: string | undefined): string | undefined {
  if (!level) return undefined;
  return level === 'xhigh' ? 'Extra high' : level.charAt(0).toUpperCase() + level.slice(1);
}

/** What the Effort row and the trigger show: "Ultracode" while this chat's
 *  switch is on (merged plan Track U), else the agent's saved level. */
export function effortShown(saved: string | undefined, ultracode: boolean): string | undefined {
  return ultracode ? 'Ultracode' : effortLabel(saved);
}

/** A row's identity, kept as the menu's highlight (ENG-81): a number would
 *  point at another model once a catalog that finishes loading while the
 *  menu is open inserts rows above it. */
export function rowKey(row: MenuRow): string {
  return `${row.typed ? 'typed' : row.custom ? 'custom' : 'listed'}:${row.agent}:${row.id}`;
}

/** Whether focus moving to `to` closes the open menu (ENG-81): back to its
 *  own button (Shift+Tab) or anywhere outside it does; within the menu, a
 *  dialog it asked (the permission confirm), a Select's popover, or nowhere
 *  (a folded list's option removed, a click on blank space) does not. The
 *  window losing focus (Cmd+Tab away) is the caller's own listener. */
export function focusLeavesMenu(
  to: { closest(selector: string): unknown } | null,
  pop: { contains(node: unknown): boolean },
  trigger: unknown,
  inPopover: (node: unknown) => boolean = () => false,
): boolean {
  if (!to) return false;
  if (to === trigger) return true;
  if (pop.contains(to) || inPopover(to)) return false;
  return !to.closest('[role="dialog"], [role="alertdialog"], dialog');
}

/** Where the highlighted row now is in `rows`: the row with `key`, else the
 *  first. */
export function rowIndexByKey(rows: readonly MenuRow[], key: string | null): number {
  const at = key == null ? -1 : rows.findIndex(r => rowKey(r) === key);
  return at < 0 ? 0 : at;
}

export function sameRow(a: MenuRow, b: { agent: string; model: string }, models?: readonly MenuModel[]): boolean {
  if (a.agent !== b.agent) return false;
  if (a.id === b.model) return true;
  // A saved level variant (`gemini-4-neon-high`) checks its base row.
  return !!a.id && !a.custom && catalogModel(models, b.model)?.id === a.id;
}
