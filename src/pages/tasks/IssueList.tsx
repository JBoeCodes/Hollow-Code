// The Tasks page's list (overhaul B1, design section 12): dense rows grouped
// by status in the team's state order (or by assignee, project or priority,
// B2's Display choice), each group collapsible with its
// count, and the vault's local tasks in their own group with no identifier.
// Every row shows its priority, identifier, status, title, labels, project,
// due date and assignee (Unassigned said as such). The page owns selection
// and the keys; a row is one button, so Tab, j/k and the arrows all land on
// the same control.
import { For, Match, Show, Switch, createMemo, createSignal, onCleanup } from 'solid-js';
import { Icon } from '../../icons';
import type { Task } from '../../types';
import { isOpenTask } from '../../taskStatus';
import { dueText, type IssueGroup } from '../../trackerTasks';
import { rowTeam, teamName } from '../../trackerTeams';
import { Avatar, PriorityIcon, StatusIcon } from './issueIcons';
import { SourceLabelChip } from '../../components/SourceLabelChip';
import { nextLimit, rowsDrawn } from './rowWindow';

export const LOCAL_GROUP = 'local';

/** The "Show more" row that ends a group drawn in steps (rowWindow.ts). The
 *  button is the way to more rows; scrolling it near the view does the same. */
function MoreRows(props: { left: number; onMore: () => void }) {
  let watch: IntersectionObserver | undefined;
  onCleanup(() => watch?.disconnect());
  return (
    <li class="issue-more">
      <button type="button" class="issue-more-btn" onClick={() => props.onMore()}
        ref={el => {
          if (typeof IntersectionObserver === 'undefined') return;
          watch = new IntersectionObserver(hits => { if (hits.some(h => h.isIntersecting)) props.onMore(); }, { rootMargin: '400px 0px' });
          watch.observe(el);
        }}>
        Show more <small>{props.left} left</small>
      </button>
    </li>
  );
}

export function IssueList(props: {
  groups: IssueGroup[];
  local: Task[];
  /** Show the local group (a view filtered to tracked issues hides it). */
  showLocal: boolean;
  selected: string | null;
  collapsed: ReadonlySet<string>;
  onToggleGroup: (id: string) => void;
  onOpen: (id: string) => void;
  onOpenLocal: (url: string) => void;
  onToggleLocal: (task: Task) => void;
  localBusy: (url: string) => string;
  /** Show each row's team key chip (two teams in the mirror). */
  showTeam?: boolean;
}) {
  // By id, so a refresh that changes nothing keeps every row's DOM (and the
  // focused row's focus); `For` keys on the id strings.
  const groupById = createMemo(() => new Map(props.groups.map(g => [g.id, g])));
  const rowById = createMemo(() => new Map(props.groups.flatMap(g => g.rows.map(r => [r.id, r] as const))));
  // Rows drawn per group (profiling night 11-20): a step at first, more on request, and always
  // through the selected row, so the keys never land on a row that is not in the page.
  const [limits, setLimits] = createSignal<Readonly<Record<string, number>>>({});
  const drawn = (g: IssueGroup) => rowsDrawn(g.rows.length, limits()[g.id] ?? null, g.rows.findIndex(r => r.id === props.selected));
  const more = (g: IssueGroup) => setLimits(l => ({ ...l, [g.id]: nextLimit(drawn(g), g.rows.length) }));
  const localDrawn = () => rowsDrawn(props.local.length, limits()[LOCAL_GROUP] ?? null, props.local.findIndex(t => t.url === props.selected));
  const moreLocal = () => setLimits(l => ({ ...l, [LOCAL_GROUP]: nextLimit(localDrawn(), props.local.length) }));
  return (
    <div class="issue-list" aria-label="Tasks">
      <For each={props.groups.map(g => g.id)}>{gid => { const group = () => groupById().get(gid)!; return (
        <Show when={groupById().get(gid)}>
        <section class="issue-group" data-kind={group().kind} data-by={group().by ?? 'status'} aria-labelledby={`issue-group-${group().id}`}>
          <h2 class="issue-group-head">
            <button id={`issue-group-${group().id}`} class="issue-group-toggle" aria-expanded={!props.collapsed.has(group().id)} onClick={() => props.onToggleGroup(group().id)}>
              <span class="issue-group-chevron" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
              <Switch fallback={<StatusIcon kind={group().kind === 'local' ? 'unstarted' : group().kind} label="" />}>
                <Match when={group().by === 'assignee'}><Avatar name={group().person} /></Match>
                <Match when={group().by === 'priority'}><PriorityIcon priority={group().priority ?? 0} /></Match>
                <Match when={group().by === 'project'}><Icon name="folder" size={14} /></Match>
              </Switch>
              <span class="issue-group-name">{group().name}</span>
              <span class="issue-group-count" aria-label={`${group().rows.length} ${group().rows.length === 1 ? 'issue' : 'issues'}`}>{group().rows.length}</span>
            </button>
          </h2>
          <Show when={!props.collapsed.has(group().id)}>
            <ul class="issue-rows">
              <For each={group().rows.slice(0, drawn(group())).map(r => r.id)}>{rid => {
                const row = () => rowById().get(rid)!;
                const due = () => dueText(row().due);
                return (
                  <li class="issue-row" data-issue-id={row().id} data-selected={props.selected === row().id ? '' : undefined}
                    data-pending={row().pending ? '' : undefined} data-conflict={row().conflict ? '' : undefined}>
                    <button class="issue-row-main" aria-current={props.selected === row().id ? 'true' : undefined} onClick={() => props.onOpen(row().id)}
                      aria-label={`${row().key} ${row().title}, ${row().statusName}, ${row().assignee ?? 'Unassigned'}${row().sourceLabel ? `, ${row().sourceLabel!.text}` : ''}`}
                      title={row().sourceLabel?.text}>
                      <PriorityIcon priority={row().priority} />
                      <span class="issue-key">{row().key}</span>
                      <Show when={rowTeam(row(), !!props.showTeam)}>{t => <span class="issue-team" data-team={t()} title={`Team ${teamName(t())}`}>{t()}</span>}</Show>
                      <StatusIcon kind={row().statusKind} label={row().statusName} />
                      <span class="issue-title">{row().title}</span>
                      <Show when={row().pending}><span class="issue-flag" title="Waiting to send to the tracker">Sending</span></Show>
                      {/* H4b: every row carries its label; it shows as a chip when it differs from Tracked (unreachable). */}
                      <Show when={row().sourceLabel && row().sourceLabel!.kind !== 'tracked'}><SourceLabelChip label={row().sourceLabel} class="issue-source" /></Show>
                      <Show when={row().conflict}><span class="issue-flag" data-tone="warning" title="The tracker changed a field you edited, so your edit was not applied. Open the issue to re-apply it (Use mine) or keep the tracker’s.">Conflict</span></Show>
                      <span class="issue-meta">
                        <For each={row().labels}>{l => <span class="issue-label">{l}</span>}</For>
                        <Show when={row().project}>{p => <span class="issue-project" title="Project">{p()}</span>}</Show>
                        <Show when={due()}>{d => <span class="issue-due" data-overdue={d().overdue ? '' : undefined} title={`Due ${row().due}`}>{d().text}</span>}</Show>
                        <span class="issue-assignee" data-unassigned={row().assignee ? undefined : ''}>
                          <Avatar name={row().assignee} />
                          <span class="issue-assignee-name">{row().assignee ?? 'Unassigned'}</span>
                        </span>
                      </span>
                    </button>
                  </li>
                );
              }}</For>
              <Show when={drawn(group()) < group().rows.length}><MoreRows left={group().rows.length - drawn(group())} onMore={() => more(group())} /></Show>
            </ul>
          </Show>
        </section>
        </Show>
      ); }}</For>
      <Show when={props.showLocal && props.local.length}>
        <section class="issue-group" data-kind="local" aria-labelledby="issue-group-local">
          <h2 class="issue-group-head">
            <button id="issue-group-local" class="issue-group-toggle" aria-expanded={!props.collapsed.has(LOCAL_GROUP)} onClick={() => props.onToggleGroup(LOCAL_GROUP)}>
              <span class="issue-group-chevron" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
              <span class="issue-group-name">Local tasks</span>
              <span class="issue-group-count" aria-label={`${props.local.length} ${props.local.length === 1 ? 'task' : 'tasks'}`}>{props.local.length}</span>
              <span class="issue-group-note">Stored in Hollow, not in the tracker</span>
            </button>
          </h2>
          <Show when={!props.collapsed.has(LOCAL_GROUP)}>
            <ul class="issue-rows">
              <For each={props.local.slice(0, localDrawn())}>{task => {
                const done = () => !isOpenTask(task.status);
                const due = () => dueText(task.due);
                return (
                  <li class="issue-row task-row" data-task-url={task.url} data-selected={props.selected === task.url ? '' : undefined} data-done={done() ? '' : undefined}>
                    <button class="issue-row-check" aria-label={done() ? `Reopen ${task.title}` : `Mark ${task.title} done`}
                      disabled={!!props.localBusy(task.url)} title={props.localBusy(task.url) || undefined} onClick={() => props.onToggleLocal(task)}>
                      <StatusIcon kind="local" done={done()} />
                    </button>
                    <button class="issue-row-main" aria-current={props.selected === task.url ? 'true' : undefined} onClick={() => props.onOpenLocal(task.url)}
                      title={task.sourceLabel?.text} aria-description={task.sourceLabel?.text}>
                      <span class="issue-title">{task.title}</span>
                      <span class="issue-meta">
                        <For each={task.labels}>{l => <span class="issue-label">{l}</span>}</For>
                        <Show when={task.projectName}>{p => <span class="issue-project" title="Project">{p()}</span>}</Show>
                        <Show when={due()}>{d => <span class="issue-due" data-overdue={d().overdue && !done() ? '' : undefined}>{d().text}</span>}</Show>
                        {/* H4b: a local task says so in its group; a row speaks only when it differs (was tracked, tracker unreachable). */}
                        <Show when={task.sourceLabel && (task.sourceLabel.kind !== 'stored' || task.sourceLabel.wasTracked)}><SourceLabelChip label={task.sourceLabel} class="issue-source" /></Show>
                      </span>
                    </button>
                  </li>
                );
              }}</For>
              <Show when={localDrawn() < props.local.length}><MoreRows left={props.local.length - localDrawn()} onMore={moreLocal} /></Show>
            </ul>
          </Show>
        </section>
      </Show>
    </div>
  );
}
