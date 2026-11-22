// Status, priority and assignee marks for the Tasks page (overhaul B1),
// drawn in Hollow's own tokens: the status ring by state kind (a tracker's
// names vary, its kinds don't), priority bars, and an initials avatar.
import { Show } from 'solid-js';
import type { TrackerStateKind } from '../../generated/ipc';
import { KIND_LABELS, initials, priorityLabel } from '../../trackerTasks';

export function StatusIcon(props: { kind: TrackerStateKind | 'local'; done?: boolean; label?: string }) {
  const kind = () => (props.kind === 'local' ? (props.done ? 'completed' : 'unstarted') : props.kind);
  // An empty label marks the ring as decoration (the state name sits beside
  // it), so it leaves the accessibility tree instead of being an unnamed img.
  const decorative = () => props.label === '';
  return (
    <svg class="issue-status-icon" data-kind={kind()} viewBox="0 0 14 14" width="14" height="14"
      role={decorative() ? undefined : 'img'} aria-hidden={decorative() ? 'true' : undefined}
      aria-label={decorative() ? undefined : props.label ?? (props.kind === 'local' ? (props.done ? 'Done' : 'Open') : KIND_LABELS[props.kind])}>
      <Show when={kind() === 'backlog'}><circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-dasharray="2 2" /></Show>
      <Show when={kind() === 'unstarted'}><circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" stroke-width="1.5" /></Show>
      <Show when={kind() === 'started'}>
        <circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" stroke-width="1.5" />
        <path d="M7 3.5 A3.5 3.5 0 0 1 7 10.5 Z" fill="currentColor" />
      </Show>
      <Show when={kind() === 'completed'}>
        <circle cx="7" cy="7" r="6.25" fill="currentColor" />
        <path d="M4.4 7.2 6.2 9 9.6 5.2" fill="none" stroke="var(--surface-card)" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </Show>
      <Show when={kind() === 'canceled'}>
        <circle cx="7" cy="7" r="6.25" fill="currentColor" />
        <path d="M5 5l4 4M9 5l-4 4" stroke="var(--surface-card)" stroke-width="1.5" stroke-linecap="round" />
      </Show>
    </svg>
  );
}

export function PriorityIcon(props: { priority: number }) {
  return (
    <svg class="issue-priority-icon" data-priority={props.priority} viewBox="0 0 14 14" width="14" height="14" role="img" aria-label={priorityLabel(props.priority)}>
      <Show when={props.priority === 1} fallback={
        <Show when={props.priority > 0} fallback={<path d="M2 7h2M6 7h2M10 7h2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />}>
          <rect x="1.5" y="8" width="3" height="4.5" rx="0.75" fill="currentColor" />
          <rect x="5.5" y="5" width="3" height="7.5" rx="0.75" fill="currentColor" opacity={props.priority <= 3 ? 1 : 0.3} />
          <rect x="9.5" y="2" width="3" height="10.5" rx="0.75" fill="currentColor" opacity={props.priority <= 2 ? 1 : 0.3} />
        </Show>
      }>
        <rect x="1" y="1" width="12" height="12" rx="3" fill="currentColor" />
        <path d="M7 3.8v4M7 10.2v.1" stroke="var(--surface-card)" stroke-width="1.8" stroke-linecap="round" />
      </Show>
    </svg>
  );
}

export function Avatar(props: { name: string | null | undefined; size?: number }) {
  return (
    <span class="issue-avatar" data-empty={props.name ? undefined : ''} style={{ '--avatar-size': `${props.size ?? 18}px` }} aria-hidden="true">
      {props.name ? initials(props.name) : ''}
    </span>
  );
}
