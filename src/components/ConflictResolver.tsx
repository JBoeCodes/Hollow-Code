import { Show, createMemo, createSignal } from 'solid-js';
import { Button, Segmented, StatePanel } from './ui';
import { DiffView } from './DiffView';
import { hasConflictMarkers, merge3, withEol } from '../textMerge';

/**
 * Three-way conflict recovery (next-level P08). A save found the file changed
 * on disk since the draft began. Nothing has been written: the base (what the
 * draft started from), the file now and the draft are all on screen, and the
 * user chooses explicitly:
 *   - keep the draft (it replaces the file; the file's changes are shown first),
 *   - use the file on disk (the draft is discarded, with that wording), or
 *   - save a merge: non-overlapping changes are combined, overlapping ones are
 *     marked and must be resolved by hand before saving.
 * Each choice still saves compare-and-swap against the disk version shown
 * here, so a second external edit is caught again rather than overwritten.
 *
 * The merge text is seeded once, so the editor mounts this keyed on the
 * conflict (base, disk, draft): a second outside edit mounts a fresh resolver
 * with a fresh merge (review finding A·D2). The merge is saved together with
 * the disk version it was built from, and the editor refuses it if that is no
 * longer the file on disk. The saved merge keeps the file's line breaks
 * (`MergeResult.eol`), even after the textarea hands it back with LF.
 */
export function ConflictResolver(props: {
  name: string;
  base: string;
  disk: string;
  draft: string;
  busy?: boolean;
  onKeepDraft: () => void;
  onUseDisk: () => void;
  onSaveMerged: (text: string, mergedAgainst: string) => void;
}) {
  const merged = createMemo(() => merge3(props.base, props.disk, props.draft));
  const mergedAgainst = props.disk;
  const [tab, setTab] = createSignal<'disk' | 'draft' | 'merge'>('disk');
  const [mergeText, setMergeText] = createSignal(merged().text);
  const unresolved = () => hasConflictMarkers(mergeText());
  return <section class="conflict-resolver" aria-label={`Conflict in ${props.name}`}>
    <StatePanel kind="conflicted" compact title="This file changed while you were editing">
      Nothing was overwritten and your draft is kept. Compare the two versions, then choose which one to save.
      {merged().conflicts ? ` ${merged().conflicts} place${merged().conflicts === 1 ? '' : 's'} changed on both sides.` : ' The changes don’t overlap, so they can be merged automatically.'}
    </StatePanel>
    <div class="conflict-tabs">
      <Segmented label="Compare" value={tab()} onChange={setTab} items={[{ value: 'disk', label: 'Changes on disk' }, { value: 'draft', label: 'Your changes' }, { value: 'merge', label: merged().conflicts ? `Merge (${merged().conflicts} to resolve)` : 'Merged' }] as const}/>
    </div>
    <Show when={tab() === 'disk'}><DiffView before={props.base} after={props.disk} beforeLabel="When you started" afterLabel="File on disk now"/></Show>
    <Show when={tab() === 'draft'}><DiffView before={props.base} after={props.draft} beforeLabel="When you started" afterLabel="Your draft"/></Show>
    <Show when={tab() === 'merge'}>
      <p class="conflict-hint">{merged().conflicts ? 'Edit the text below. Each conflict is between “<<<<<<< Your draft” and “>>>>>>> File on disk”; keep what you want and delete the markers.' : 'Both sets of changes, combined. Review before saving.'}</p>
      <textarea class="conflict-merge" aria-label="Merged text" spellcheck={false} value={mergeText()} onInput={e => setMergeText(e.currentTarget.value)}/>
    </Show>
    <div class="conflict-actions">
      <Show when={tab() === 'merge'} fallback={<Button variant="primary" disabled={props.busy} onClick={() => setTab('merge')}>{merged().conflicts ? 'Resolve and merge…' : 'Review the merge…'}</Button>}>
        <Button variant="primary" disabled={props.busy || unresolved()} title={unresolved() ? 'Remove the conflict markers first' : undefined} onClick={() => props.onSaveMerged(withEol(mergeText(), merged().eol), mergedAgainst)}>Save merged version</Button>
      </Show>
      <Button disabled={props.busy} onClick={props.onKeepDraft}>Keep my draft (replace the file on disk)</Button>
      <Button variant="ghost" disabled={props.busy} onClick={props.onUseDisk}>Use the file on disk (discard my draft)</Button>
    </div>
  </section>;
}
