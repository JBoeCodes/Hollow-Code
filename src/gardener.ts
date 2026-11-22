// The gardener's review card (master plan #19, `src-tauri/src/gardener.rs`):
// pure helpers for its lines and the person's ticks. Apply ticked sends only the
// ticked ids; the backend re-scans and applies only those it still finds.

export type GardenAction = 'remove-worktree' | 'archive' | 'trash' | 'manual' | 'resolve';

export interface GardenLine {
  id: string;
  kind: string;
  target: string;
  title: string;
  reason: string;
  action: GardenAction;
  undo: string;
}

const ACTIONS: readonly GardenAction[] = ['remove-worktree', 'archive', 'trash', 'manual', 'resolve'];
/** Lines Hollow never applies: the person's own to do. */
const PERSONS: readonly GardenAction[] = ['manual', 'resolve'];

/** The card's lines, from its payload; malformed entries are left out. */
export function gardenLines(payload: Record<string, unknown>): GardenLine[] {
  const items = payload.items;
  if (!Array.isArray(items)) return [];
  return items.flatMap(raw => {
    if (!raw || typeof raw !== 'object') return [];
    const r = raw as Record<string, unknown>;
    const s = (k: string) => (typeof r[k] === 'string' ? (r[k] as string) : '');
    const action = s('action') as GardenAction;
    if (!s('id') || !ACTIONS.includes(action)) return [];
    return [{ id: s('id'), kind: s('kind'), target: s('target'), title: s('title'), reason: s('reason'), action, undo: s('undo') }];
  });
}

/** Ticked by default: every line Hollow can apply (never a manual or a resolve one). */
export function defaultTicked(lines: GardenLine[]): string[] {
  return lines.filter(l => !PERSONS.includes(l.action)).map(l => l.id);
}

/** The edited payload Apply ticked sends: the ticked, applicable ids. */
export function gardenSelection(lines: GardenLine[], ticked: readonly string[]): { ticked: string[] } {
  const allowed = new Set(defaultTicked(lines));
  return { ticked: lines.map(l => l.id).filter(id => allowed.has(id) && ticked.includes(id)) };
}

/** The action's plain words. */
export function gardenActionLabel(action: GardenAction): string {
  switch (action) {
    case 'remove-worktree': return 'Remove worktree';
    case 'archive': return 'Archive';
    case 'trash': return 'Move to trash';
    case 'manual': return 'By hand in Notion';
    case 'resolve': return 'Resolve in Needs you';
  }
}
