/** Live pins are local presentation preferences. Markdown and saved revisions stay unchanged. */
export interface CanvasPin {
  vault: string;
  workspaceId: string | null;
  sessionKey: string | null;
  id: string;
  revision: number;
  sha256: string;
  title: string;
  destination: 'home' | 'workspace';
}
export const MAX_PINS = 12;
export const BINDINGS = ['plan.done', 'plan.open', 'plan.total', 'tasks.open', 'tasks.done', 'tasks.total', 'usage.runs', 'usage.reportedUsd', 'sessions.running'] as const;
export type Binding = typeof BINDINGS[number];
export type LiveValues = Partial<Record<Binding, number | null>>;
export const pinKey = (p: Pick<CanvasPin, 'vault' | 'workspaceId' | 'sessionKey' | 'id' | 'destination'>) => JSON.stringify([p.vault, p.workspaceId, p.sessionKey, p.id, p.destination]);

export function decodePins(raw: string | null): CanvasPin[] {
  try {
    const list: unknown = JSON.parse(raw ?? '[]'); if (!Array.isArray(list)) return [];
    const seen = new Set<string>();
    return list.filter((p): p is CanvasPin => {
      if (!p || typeof p !== 'object') return false;
      const v = p as CanvasPin;
      if (typeof v.vault !== 'string' || !v.vault.startsWith('/') || v.vault.length > 4096 ||
        !((typeof v.workspaceId === 'string' && /^[A-Za-z0-9_-]{1,180}$/.test(v.workspaceId) && v.sessionKey === null) ||
          (v.workspaceId === null && typeof v.sessionKey === 'string' && /^[A-Za-z0-9_-]{1,180}$/.test(v.sessionKey))) ||
        typeof v.id !== 'string' || !/^(?:[a-z0-9-]{1,64}|_(?:overview|brief|plan|sources))$/.test(v.id) ||
        !Number.isSafeInteger(v.revision) || v.revision < 1 || typeof v.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(v.sha256) ||
        typeof v.title !== 'string' || !v.title.trim() || v.title.length > 720 ||
        !['home', 'workspace'].includes(v.destination) || (v.destination === 'workspace' && !v.workspaceId)) return false;
      const key = pinKey(v); if (seen.has(key)) return false; seen.add(key); return true;
    }).slice(0, MAX_PINS);
  } catch { return []; }
}

export function taskValues(tasks: readonly { projectUrl: string | null; bucket: string }[], project: string): LiveValues {
  const rows = project ? tasks.filter(t => t.projectUrl === project) : tasks;
  const done = rows.filter(t => t.bucket === 'done').length;
  return { 'tasks.open': rows.length - done, 'tasks.done': done, 'tasks.total': rows.length };
}
export function usageValues(t: { reportedCostUsd: number | null; runs: number; partial: number; unknown: number; inProgress: number; unpriced?: number }): LiveValues {
  return { 'usage.runs': t.runs, 'usage.reportedUsd': t.unknown || t.partial || t.inProgress || t.unpriced ? null : t.reportedCostUsd };
}

/** Share concurrent reads, never cache a settled result across a vault change. */
export function sharedReads() {
  const reads = new Map<string, Promise<unknown>>();
  return <T>(key: string, read: () => Promise<T>): Promise<T> => {
    const existing = reads.get(key); if (existing) return existing as Promise<T>;
    const pending = Promise.resolve().then(read).finally(() => { if (reads.get(key) === pending) reads.delete(key); });
    reads.set(key, pending); return pending;
  };
}

/** At most one read in flight. Invalidation during a read schedules one successor.
 * Hidden/disposed owners never receive results and hidden owners never start work. */
export function refreshLoop<T>(read: () => Promise<T>, success: (value: T) => void, failure: (error: unknown) => void) {
  let shown = false, dead = false, busy = false, dirty = true;
  async function drain() {
    if (dead || !shown || busy || !dirty) return;
    busy = true; dirty = false;
    try { const value = await read(); if (!dead && shown) success(value); }
    catch (e) { if (!dead && shown) failure(e); }
    finally { busy = false; if (!dead && shown && dirty) void drain(); }
  }
  return {
    request() { dirty = true; void drain(); },
    visible(value: boolean) { if (shown === value) return; shown = value; dirty = true; void drain(); },
    dispose() { dead = true; },
  };
}
