import { For, Show, createMemo, createSignal } from 'solid-js';
import { blockerCandidates, blockerLabel, type BlockerTask } from '../taskStructure';
import '../styles/blocked-by-picker.css';

/**
 * "Blocked by" picker (Paperclip PR 11b): removable chips for the chosen
 * blockers and a search box that offers open tasks by title. The candidate
 * list (`blockerCandidates`) leaves out the task itself, tasks already
 * chosen, finished tasks and any task that would close a loop; recurring and
 * cancelled tasks come last with a note because they never resolve on their
 * own. The backend re-checks every new entry on save (`TASK_BLOCKER`,
 * `TASK_CYCLE`), so this list is guidance, not the guard.
 */
export function BlockedByPicker(props: {
  /** Every task the list knows (the picker reads url, title, status,
   *  recurrence and blockedBy). */
  tasks: () => readonly BlockerTask[];
  /** The task being edited, or null for a new task. */
  selfUrl: string | null;
  value: () => string[];
  onChange: (next: string[]) => void;
  disabled?: boolean;
}) {
  const [query, setQuery] = createSignal('');
  const candidates = createMemo(() =>
    query().trim() ? blockerCandidates(props.tasks(), props.selfUrl, props.value(), query()) : [],
  );
  const add = (url: string) => {
    props.onChange([...props.value(), url]);
    setQuery('');
  };
  return (
    <div class="blocked-by-picker">
      <Show when={props.value().length}>
        <ul class="blocked-by-chips" aria-label="Chosen blockers">
          <For each={props.value()}>
            {url => (
              <li class="blocked-by-chip">
                <span>{blockerLabel(props.tasks(), url)}</span>
                <button
                  type="button"
                  aria-label={`Remove blocker ${blockerLabel(props.tasks(), url)}`}
                  disabled={props.disabled}
                  onClick={() => props.onChange(props.value().filter(u => u !== url))}
                >×</button>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <input
        type="search"
        aria-label="Find a task this waits on"
        placeholder="Find a task this waits on…"
        autocomplete="off"
        disabled={props.disabled}
        value={query()}
        onInput={e => setQuery(e.currentTarget.value)}
        onKeyDown={e => {
          if (e.key === 'Enter') {
            // Enter adds the best match instead of submitting the form.
            e.preventDefault();
            const first = candidates()[0];
            if (first) add(first.task.url);
          }
        }}
      />
      <Show when={query().trim()}>
        <ul class="blocked-by-options" aria-label="Matching tasks">
          <For each={candidates()} fallback={<li class="blocked-by-empty">No open task matches, or it would create a loop.</li>}>
            {c => (
              <li>
                <button type="button" disabled={props.disabled} onClick={() => add(c.task.url)}>
                  <span>{c.task.title}</span>
                  <Show when={c.note}>{note => <small>{note()}</small>}</Show>
                </button>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
}
