// A project's record (overhaul B3, board S4): the words and edits behind
// `pages/tasks/ProjectDetail.tsx`. The tracker owns a tracked project's name,
// status, lead, dates and description; the vault owns the portfolio fields in
// its PROJECT.md (the tracker design's section 5), which edit here and are
// never sent anywhere. Backend: `src-tauri/src/project_portfolio.rs`.

import type { ProjectPortfolio, TrackerProjectRow, TrackerViewRow } from './generated/ipc';

export type PortfolioField = { key: string; label: string; kind: 'text' | 'number' | 'url' | 'list'; hint?: string };

/** In the order the detail shows them. */
export const PORTFOLIO_FIELDS: readonly PortfolioField[] = [
  { key: 'department', label: 'Department', kind: 'text' },
  { key: 'type', label: 'Type', kind: 'text' },
  { key: 'hours_saved_per_year', label: 'Hours saved a year', kind: 'number' },
  { key: 'value_per_year', label: 'Value a year', kind: 'number' },
  { key: 'owners_note', label: 'Owners', kind: 'text', hint: 'People who aren’t in the tracker team' },
  { key: 'requested_by', label: 'Requested by', kind: 'list', hint: 'People records, one per line' },
  { key: 'stakeholders', label: 'Stakeholders', kind: 'list' },
  { key: 'domains', label: 'Domains', kind: 'list', hint: 'Domain records, one per line' },
  { key: 'github_url', label: 'GitHub', kind: 'url' },
  { key: 'live_url', label: 'Live site', kind: 'url' },
  { key: 'meetings', label: 'Meetings', kind: 'list', hint: 'Meeting records, one per line' },
  { key: 'captures', label: 'Captures', kind: 'list' },
  { key: 'war_room', label: 'War Room', kind: 'list', hint: 'War Room use cases, one per line' },
  { key: 'docs', label: 'Docs', kind: 'list' },
];

export type PortfolioDraft = Record<string, string>;

/** The editor's starting text: list fields one item per line. */
export function draftOf(p: ProjectPortfolio): PortfolioDraft {
  return Object.fromEntries(PORTFOLIO_FIELDS.map(f => [f.key, f.kind === 'list' ? (p.lists[f.key] ?? []).join('\n') : (p.text[f.key] ?? '')]));
}

export const listOf = (text: string) => text.split('\n').map(s => s.trim()).filter(Boolean);

/** Only what changed, in the shape `set_project_portfolio` takes. */
export function changedValues(p: ProjectPortfolio, draft: PortfolioDraft): Record<string, string | string[]> {
  const out: Record<string, string | string[]> = {};
  for (const f of PORTFOLIO_FIELDS) {
    const value = draft[f.key] ?? '';
    if (f.kind === 'list') {
      const next = listOf(value);
      const was = p.lists[f.key] ?? [];
      if (next.length !== was.length || next.some((x, i) => x !== was[i])) out[f.key] = next;
    } else if (value.trim() !== (p.text[f.key] ?? '').trim()) out[f.key] = value.trim();
  }
  return out;
}

/** Why a draft can't be saved yet, field by field (the backend says the same). */
export function draftProblems(draft: PortfolioDraft): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of PORTFOLIO_FIELDS) {
    const v = (draft[f.key] ?? '').trim();
    if (!v) continue;
    if (f.kind === 'number' && !Number.isFinite(Number(v.replace(/[,$\s]/g, '')))) out[f.key] = 'A number, like 1,200';
    if (f.kind === 'url' && !/^https?:\/\//.test(v)) out[f.key] = 'A web address starting https://';
  }
  return out;
}

/** A list item that is a vault path opens in Library; anything else is text. */
export const isVaultPath = (item: string) => /^(library|workspaces|tasks|outputs)\/[^\s]+\.md$/.test(item);

/** "library/records/people/dana-kim.md" reads as "dana kim". */
export function itemLabel(item: string): string {
  if (!isVaultPath(item)) return item;
  return (item.split('/').pop() ?? item).replace(/\.md$/, '').replace(/^\d{4}-\d{2}-\d{2}-/, '').replace(/[-_]+/g, ' ');
}

export interface ProjectCounts { total: number; done: number; open: number; canceled: number; progress: number | null }

/** Issue counts by state kind, from the mirror's rows: progress is
 *  completed over all of the project's issues (board S3c). */
export function projectCounts(rows: readonly TrackerViewRow[]): ProjectCounts {
  const done = rows.filter(r => r.statusKind === 'completed').length;
  const canceled = rows.filter(r => r.statusKind === 'canceled').length;
  const total = rows.length;
  return { total, done, canceled, open: total - done - canceled, progress: total > 0 ? done / total : null };
}

/** "40%"; null for no issues. */
export const percent = (p: number | null | undefined) => (p == null ? null : `${Math.round(p * 100)}%`);

/** The tracker's project status words by kind, for the status mark. */
export const PROJECT_KIND_LABELS: Record<string, string> = {
  backlog: 'Backlog', planned: 'Planned', started: 'In progress', paused: 'Paused', completed: 'Completed', canceled: 'Canceled',
};

/** The title a project shows: the tracker's when tracked, the vault's otherwise. */
export const projectTitle = (row: TrackerProjectRow | null | undefined, p: ProjectPortfolio | null | undefined) =>
  row?.name ?? p?.title ?? 'Project';
