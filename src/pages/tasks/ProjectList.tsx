// The Tasks page's Projects tab (overhaul B4, board S3c), laid out like a
// tracker's project list: one dense row per team project with its status,
// lead, target date, progress (completed over all its issues, from the
// mirror's rows) and open and total issues; active projects first, closed
// ones last. A row opens the project detail (`ProjectDetail.tsx`) beside the
// list, with its issues.
import { For, Show, createMemo } from 'solid-js';
import type { TrackerProjectRow, TrackerViewRow } from '../../generated/ipc';
import { percent, projectCounts } from '../../projectPortfolio';
import { dueText } from '../../trackerTasks';
import { rowTeam, teamName } from '../../trackerTeams';
import { Avatar } from './issueIcons';

/** Status kinds in the order a tracker lists projects. */
const PROJECT_KIND_ORDER = ['started', 'planned', 'backlog', 'paused', 'completed', 'canceled'];
const kindRank = (k: string) => { const i = PROJECT_KIND_ORDER.indexOf(k); return i < 0 ? PROJECT_KIND_ORDER.length : i; };

export function orderProjects(projects: readonly TrackerProjectRow[]): TrackerProjectRow[] {
  return [...projects].sort((a, b) => kindRank(a.statusKind) - kindRank(b.statusKind) || a.name.localeCompare(b.name));
}

export function ProjectList(props: {
  projects: readonly TrackerProjectRow[];
  rows: readonly TrackerViewRow[];
  selected: string | null;
  onOpen: (id: string) => void;
  /** Show each project's team key chip (two teams in the mirror). */
  showTeam?: boolean;
}) {
  const ordered = createMemo(() => orderProjects(props.projects));
  const byProject = createMemo(() => {
    const m = new Map<string, TrackerViewRow[]>();
    for (const r of props.rows) {
      if (!r.projectId) continue;
      const list = m.get(r.projectId);
      if (list) list.push(r); else m.set(r.projectId, [r]);
    }
    return m;
  });
  return (
    <div class="issue-list project-list">
      <div class="project-list-head" aria-hidden="true">
        <span>Name</span><span>Status</span><span>Lead</span><span>Target</span><span>Progress</span><span>Issues</span>
      </div>
      <ul class="issue-rows" aria-label="Projects">
        <For each={ordered()}>{p => {
          const counts = () => projectCounts(byProject().get(p.id) ?? []);
          const target = () => dueText(p.targetDate);
          return (
            <li class="issue-row project-row" data-project-id={p.id} data-kind={p.statusKind} data-selected={props.selected === p.id ? '' : undefined}>
              <button class="issue-row-main project-row-main" aria-current={props.selected === p.id ? 'true' : undefined} onClick={() => props.onOpen(p.id)}
                aria-label={`${p.name}, ${p.statusName}, ${p.lead ?? 'no lead'}, ${percent(counts().progress) ?? 'no issues'}`}>
                <span class="issue-title project-name">{p.name}<Show when={p.key}>{k => <span class="issue-key project-key">{k()}</span>}</Show><Show when={rowTeam(p, !!props.showTeam)}>{t => <span class="issue-team" data-team={t()} title={`Team ${teamName(t())}`}>{t()}</span>}</Show></span>
                <span class="project-cell"><span class="project-status-dot" data-kind={p.statusKind} aria-hidden="true" />{p.statusName}</span>
                <span class="project-cell issue-assignee" data-unassigned={p.lead ? undefined : ''}><Avatar name={p.lead} /><span class="issue-assignee-name">{p.lead ?? 'No lead'}</span></span>
                <span class="project-cell issue-due" data-overdue={target()?.overdue && p.statusKind !== 'completed' ? '' : undefined}>{target()?.text ?? 'No target'}</span>
                <span class="project-cell project-progress">
                  <span class="project-progress-bar" aria-hidden="true"><span style={{ width: `${Math.round((counts().progress ?? 0) * 100)}%` }} /></span>
                  {percent(counts().progress) ?? 'No issues'}
                </span>
                <span class="project-cell project-issue-count">{counts().open} open of {counts().total}</span>
              </button>
            </li>
          );
        }}</For>
      </ul>
    </div>
  );
}
