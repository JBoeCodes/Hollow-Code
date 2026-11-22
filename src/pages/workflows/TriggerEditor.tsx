import { For, Show, createSignal } from 'solid-js';
import { Select } from '../../components/Select';
import { ErrorText } from '../../components/ui';

/** Typed local triggers; existing unknown tokens stay until explicitly removed. */
export function TriggerEditor(props: { value: string[]; onChange: (value: string[]) => void }) {
  const [kind, setKind] = createSignal('task-overdue');
  const [glob, setGlob] = createSignal('');
  const [error, setError] = createSignal('');
  function add() {
    const token = kind() === 'file-changed' ? `file-changed:${glob().trim()}` : kind();
    if (kind() === 'file-changed' && (!glob().trim() || /[,\r\n]/.test(glob()))) {
      setError('Enter one file pattern without commas or line breaks.'); return;
    }
    if (props.value.includes(token)) { setError('This trigger is already in the list.'); return; }
    setError('');
    props.onChange([...props.value, token]);
  }
  return <div class="workflow-triggers">
    <p class="form-label">Triggers</p>
    <Show when={props.value.length} fallback={<p class="workflow-hint">No event triggers.</p>}>
      <ul><For each={props.value}>{token => <li>
        <span>{token}</span>{' '}
        <button type="button" class="btn small ghost" aria-label={`Remove ${token}`}
          onClick={() => props.onChange(props.value.filter(value => value !== token))}>Remove</button>
      </li>}</For></ul>
    </Show>
    <div class="schedule-picker-row">
      <Select label="Trigger kind" size="sm" searchable={false} value={kind()} onChange={value => { setKind(value); setError(''); }}
        options={[{ value: 'task-overdue', label: 'Task overdue' }, { value: 'task-unblocked', label: 'Task unblocked' }, { value: 'file-changed', label: 'File changed' }]} />
      <Show when={kind() === 'file-changed'}><label>File pattern <input value={glob()} onInput={e => setGlob(e.currentTarget.value)} placeholder="notes/**" /></label></Show>
      <button type="button" class="btn small" onClick={add}>Add trigger</button>
    </div>
    <Show when={error()}><ErrorText class="workflow-error" text={error()} /></Show>
  </div>;
}
