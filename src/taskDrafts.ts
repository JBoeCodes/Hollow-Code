/** One session draft envelope per vault identity; saving mapped fields must not
 * erase a sheet's local labels/steps. Baselines stay fixed until explicit review. */
export type DraftValues = Record<string, unknown>;
export interface TaskDraft<T = DraftValues> { draft: T; base: T; hash?: string | null; id?: string | null }
type Sections = { local?: TaskDraft; mapped?: TaskDraft };
export function taskDraftKey(url: string): string {
  return `hollow.task-drafts.${url.replace(/^vault:\/*/, '').replace(/^\/+/, '')}`;
}
export function readTaskDraft<T>(url: string, section: keyof Sections): TaskDraft<T> | null {
  try {
    const value = JSON.parse(sessionStorage.getItem(taskDraftKey(url)) ?? '{}')?.[section];
    return value && value.draft && typeof value.draft === 'object' && !Array.isArray(value.draft)
      && value.base && typeof value.base === 'object' && !Array.isArray(value.base) ? value : null;
  }
  catch { return null; }
}
export function writeTaskDraft<T>(url: string, section: keyof Sections, value: TaskDraft<T> | null): void {
  const key = taskDraftKey(url);
  let saved: Sections = {};
  try { const parsed = JSON.parse(sessionStorage.getItem(key) ?? '{}'); if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) saved = parsed; } catch { /* retain valid new work */ }
  if (value) saved[section] = value as TaskDraft;
  else delete saved[section];
  if (Object.keys(saved).length) sessionStorage.setItem(key, JSON.stringify(saved));
  else sessionStorage.removeItem(key);
}
export function sameDraftValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object' || Array.isArray(a) !== Array.isArray(b)) return false;
  const left = Object.keys(a), right = Object.keys(b);
  return left.length === right.length && left.every(k => Object.prototype.hasOwnProperty.call(b, k) && sameDraftValue((a as DraftValues)[k], (b as DraftValues)[k]));
}
/** Three-way rebase: untouched fields follow current; overlapping edits need a
 * choice. Only current keys survive (mapped schema changes cannot be forced). */
export function draftConflictKeys(base: DraftValues, mine: DraftValues, current: DraftValues): string[] {
  return Object.keys(current).filter(k => !sameDraftValue(mine[k], base[k]) && !sameDraftValue(current[k], base[k]) && !sameDraftValue(mine[k], current[k]));
}
export function rebaseTaskDraft<T extends DraftValues>(base: T, mine: T, current: T, choices: Record<string, 'mine' | 'current'>): T {
  return Object.fromEntries(Object.keys(current).map(k => [k, choices[k] === 'mine' ? mine[k] : choices[k] === 'current' || sameDraftValue(mine[k], base[k]) ? current[k] : mine[k]])) as T;
}
