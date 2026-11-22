import { For, Show, createSignal, onMount } from 'solid-js';
import { draftConflictKeys, rebaseTaskDraft, sameDraftValue, type DraftValues } from '../taskDrafts';
import '../styles/task-draft-review.css';

/** Mounted against an immutable current snapshot, not a polling resource. */
export function TaskDraftReview(props: {
  base: DraftValues; mine: DraftValues; current: DraftValues;
  labels?: Record<string, string>; onApply: (values: DraftValues) => void; onCancel: () => void;
}) {
  const [choices, setChoices] = createSignal<Record<string, 'mine' | 'current'>>({});
  const conflicts = () => draftConflictKeys(props.base, props.mine, props.current);
  const changed = () => Object.keys(props.current).filter(k => !sameDraftValue(props.mine[k], props.current[k]));
  const label = (key: string) => props.labels?.[key] ?? key;
  const display = (v: unknown): string => v == null || v === '' ? 'Empty' : typeof v === 'string' ? v : Array.isArray(v) ? v.map(x => x && typeof x === 'object' && 'text' in x ? `${x.done ? 'Completed' : 'Open'}: ${x.text}` : display(x)).join('; ') || 'Empty' : typeof v === 'object' && 'start' in v ? String(v.start) : JSON.stringify(v);
  const selected = (key: string) => choices()[key] ?? (conflicts().includes(key) ? undefined : sameDraftValue(props.mine[key], props.base[key]) ? 'current' : 'mine');
  const missing = () => conflicts().filter(k => !choices()[k]);
  const removed = () => Object.keys(props.mine).filter(k => !(k in props.current) && !sameDraftValue(props.mine[k], props.base[k]));
  let heading: HTMLHeadingElement | undefined;
  onMount(() => heading?.focus());
  return <section class="task-draft-review" aria-label="Review task changes">
    <h3 ref={heading} tabIndex={-1}>Review task changes</h3><p>Your draft is kept. Choose which value to keep where both changed. Unedited fields follow the current task.</p>
    <For each={removed()}>{key=><p>No longer synced: {label(key)}. Your draft value: {display(props.mine[key])}. Applying this review removes that field from the draft.</p>}</For>
    <For each={changed()} fallback={<p>The values match; review updates the saved baseline.</p>}>{key => <div class="task-detail-conflict">
      <h4>{label(key)}</h4><p>Mine: {display(props.mine[key])}</p><p>Current: {display(props.current[key])}</p>
      <Show when={conflicts().includes(key)}><p>Both changed. Choose one.</p></Show>
      <div class="work-actions">
        <button type="button" class="btn small" aria-label={`Use mine for ${label(key)}`} aria-pressed={selected(key) === 'mine'} onClick={() => setChoices(c => ({ ...c, [key]: 'mine' }))}>Use mine</button>
        <button type="button" class="btn small" aria-label={`Use current for ${label(key)}`} aria-pressed={selected(key) === 'current'} onClick={() => setChoices(c => ({ ...c, [key]: 'current' }))}>Use current</button>
      </div>
    </div>}</For>
    <Show when={missing().length}><p role="status">Choose a value for: {missing().map(label).join(', ')}.</p></Show>
    <div class="work-actions"><button type="button" class="btn small primary" disabled={!!missing().length} onClick={() => props.onApply(rebaseTaskDraft(props.base, props.mine, props.current, choices()))}>Use selected values</button><button type="button" class="btn small ghost" onClick={props.onCancel}>Back to draft</button></div>
  </section>;
}
