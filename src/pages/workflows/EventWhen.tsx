import { Show, createResource, createSignal, type JSX } from 'solid-js';
import { Select } from '../../components/Select';
import { ErrorText } from '../../components/ui';
import { projectWorkspaces } from '../../store/workspaces';
import { EVENT_KINDS, eventToken, eventTriggersStatus, setEventTrigger, tokenFor, type EventKind } from '../../eventTriggersApi';

/** An automation's When (COO rebuild F2): a Schedule (the picker and local
 *  triggers in `children`), or an Event: a new issue in team AI, a merged PR
 *  in a connected repo, or a recording or meeting ending. An event is saved
 *  as one `triggers:` token (`tracker-issue:<project>`), with the schedule
 *  manual; saving turns that trigger on (`eventTriggersApi.turnOnFor`). */
export function EventWhen(props: {
  triggers: string[];
  onTriggers: (value: string[]) => void;
  onSchedule: (value: string) => void;
  /** The automation's own project, the default for a per-project event. */
  workspaceId: string | null;
  children: JSX.Element;
}) {
  const [picked, setPicked] = createSignal<'schedule' | 'event' | null>(null);
  const token = () => eventToken(props.triggers);
  const mode = () => picked() ?? (token() ? 'event' : 'schedule');
  const [status, { mutate }] = createResource(() => mode() === 'event', () => eventTriggersStatus().catch(() => null));
  const [error, setError] = createSignal('');
  const kind = () => token()?.[0] ?? 'tracker-issue';
  const workspace = () => token()?.[1] ?? null;
  const meta = () => EVENT_KINDS.find(k => k.kind === kind())!;
  const kindStatus = () => status()?.kinds.find(k => k.kind === kind());
  const workspaceOn = () => !!kindStatus()?.workspaces.some(w => w.id === workspace());
  /** Turn off is shared: say what else it stops, and what it leaves on. */
  const sharedText = () => {
    const parts = [meta().perWorkspace
      ? `Turn off stops this event for every automation on ${projectTitle()}.`
      : 'Turn off stops this event for every automation on it.'];
    if (kind() === 'tracker-issue' && workspace() && status()?.leadWakes.includes(workspace()!)) {
      parts.push('The project\'s lead keeps waking for new issues; its menu turns that off.');
    }
    return parts.join(' ');
  };

  /** An event automation has exactly one trigger: the event. */
  function choose(next: EventKind, ws: string | null) {
    const per = EVENT_KINDS.find(k => k.kind === next)?.perWorkspace;
    props.onTriggers([tokenFor(next, per ? (ws ?? props.workspaceId) : null)]);
  }
  function toEvent() {
    setPicked('event');
    props.onSchedule('manual');
    if (!token()) choose('tracker-issue', props.workspaceId);
  }
  function toSchedule() {
    setPicked('schedule');
    props.onTriggers(props.triggers.filter(t => !eventToken([t])));
  }
  async function turnOff() {
    setError('');
    try {
      mutate(await setEventTrigger(meta().perWorkspace && workspace() ? { kind: kind(), workspaceId: workspace()!, on: false } : { kind: kind(), on: false }));
    } catch (e) { setError(String(e)); }
  }
  const sinceText = (at: string | null | undefined) => at ? new Date(at).toLocaleString([], { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }) : '';
  const projectTitle = () => projectWorkspaces().find(w => w.id === workspace())?.title ?? workspace() ?? 'this project';

  return <div class="workflow-when">
    <div class="schedule-picker-row" role="group" aria-label="When">
      <span class="form-label">When</span>
      <button type="button" class="btn small" classList={{ primary: mode() === 'schedule' }} aria-pressed={mode() === 'schedule'} onClick={toSchedule}>Schedule</button>
      <button type="button" class="btn small" classList={{ primary: mode() === 'event' }} aria-pressed={mode() === 'event'} onClick={toEvent}>Event</button>
    </div>
    <Show when={mode() === 'event'} fallback={props.children}>
      <div class="schedule-picker-row">
        <Select label="Event" size="sm" searchable={false} value={kind()} onChange={v => choose(v as EventKind, workspace())}
          options={EVENT_KINDS.map(k => ({ value: k.kind, label: k.label }))} />
        <Show when={meta().perWorkspace}>
          <Select label="Event project" size="sm" placeholder="Choose a project…" value={workspace() ?? ''} onChange={v => choose(kind(), v)}
            emptyText="No project matches" options={projectWorkspaces().map(w => ({ value: w.id, label: w.title }))} />
        </Show>
      </div>
      <p class="workflow-hint" role="status">
        <Show when={kindStatus()} fallback="Event triggers are off until turned on; saving turns this one on. Events from before never start it.">
          {k => <Show when={k().on && (!meta().perWorkspace || workspaceOn())}
            fallback={`Off. Saving turns it on${meta().perWorkspace ? ` for ${projectTitle()}` : ''}; events from before never start it. Up to ${k().capPerHour} runs an hour.`}>
            {`On since ${sinceText(k().perWorkspace ? k().workspaces.find(w => w.id === workspace())?.since : k().since)} · up to ${k().capPerHour} runs an hour (${k().firedLastHour} this hour). `}
            <button type="button" class="btn small ghost" onClick={() => void turnOff()}>Turn off</button>
            {` ${sharedText()}`}
          </Show>}
        </Show>
      </p>
      <Show when={kind() === 'pr-merged' && status() && workspace() && !status()!.connected.includes(workspace()!)}>
        <p class="workflow-hint">This project has no connected repository: choose a build workspace.</p>
      </Show>
      <Show when={error()}><ErrorText class="workflow-error" text={error()} /></Show>
    </Show>
  </div>;
}
