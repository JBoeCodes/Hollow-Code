import '../styles/office-preview.css';
import { For, Match, Show, Switch, Suspense, createResource, lazy, type JSX } from 'solid-js';
import { invoke } from '../api';
import { unlessErrored } from '../stableResource';
import { PlainErrorText } from './ui';
import { Icon } from '../icons';

export type OfficePart = { title: string; paragraphs: string[]; columns: string[]; rows: { number: number; cells: string[]; formulas?: string[] }[] };
// UN1: the live sheet loads with the first Excel file opened.
const SheetGrid = lazy(() => import('./SheetGrid'));
export type OfficePreviewData = { kind: 'word' | 'excel' | 'powerpoint'; parts: OfficePart[]; truncated: string[] };

/** Preview pane's read-only text preview of Word, Excel and PowerPoint files
 * (B1b, ENG-104), built from `office_preview`, and the generic binary fallback. */
export function OfficePreview(props: { path: string; nonce?: number; sessionKey?: string | null; onOpen: (reveal: boolean) => void }): JSX.Element {
  const office = () => /\.(docx|xlsx|pptx)$/i.test(props.path);
  const [read, { refetch }] = createResource(
    () => office() ? { path: props.path, nonce: props.nonce ?? 0, session: props.sessionKey ?? null } : null,
    async s => ({ ...s, value: await invoke<OfficePreviewData>('office_preview', { path: s.path, sessionKey: s.session }) }),
  );
  const current = () => {
    const data = unlessErrored(read);
    return data?.path === props.path && data.session === (props.sessionKey ?? null) ? data.value : null;
  };
  const excel = () => /\.xlsx$/i.test(props.path);
  const name = () => props.path.slice(props.path.lastIndexOf('/') + 1);
  const label = () => /\.docx$/i.test(props.path) ? 'Word' : /\.xlsx$/i.test(props.path) ? 'Excel' : 'PowerPoint';
  const icon = () => /\.docx$/i.test(props.path) ? 'fileType' : /\.xlsx$/i.test(props.path) ? 'fileSpreadsheet' : 'presentation';
  return <Show when={office()} fallback={
    <div class="empty-state chat-preview-empty">
      <p>No inline preview for this file type.</p>
      <button class="btn small" onClick={() => props.onOpen(false)}>Open in default app</button>
      <button class="btn small ghost" onClick={() => props.onOpen(true)}>Reveal in Finder</button>
    </div>
  }>
    <section class="office-preview" aria-label={`${label()} preview: ${name()}`} aria-busy={read.loading}>
      <div class="office-preview-header">
        <Icon name={icon()} size={16} />
        <strong class="office-preview-name">{name()}</strong>
        <span class="office-preview-kind">{label()}</span>
        <p class="office-preview-hint">{excel() ? 'Read-only sheet with formulas; open it in its app to edit.' : 'Preview of the text only; open it in its app for layout.'}</p>
        <button type="button" class="btn small" onClick={() => props.onOpen(false)}>Open in app</button>
      </div>
      <Show when={read.loading && !current()}>
        <p class="office-preview-message" role="status">Reading {name()}…</p>
      </Show>
      <Show when={read.error && !read.loading}>
        <div class="office-preview-message" role="alert">
          <strong>Couldn’t preview this file</strong>
          <PlainErrorText text={String(read.error)} />
          <span><button type="button" class="btn small" onClick={() => void refetch()}>Retry</button>{' '}
            <button type="button" class="btn small ghost" onClick={() => props.onOpen(true)}>Reveal in Finder</button></span>
        </div>
      </Show>
      <Show when={current()}>{data => <>
        <Show when={data().truncated.length}>
          <p class="office-preview-truncated" role="note">Shortened: {data().truncated.join('; ')}. Open it in its app for everything.</p>
        </Show>
        <div class="office-preview-body">
          <Show when={data().parts.length && !(data().kind === 'word' && data().parts.length === 1 && !data().parts[0].paragraphs.length)} fallback={<p class="office-preview-empty">No text found in this file.</p>}>
            <Switch>
              <Match when={data().kind === 'word'}>
                <article class="office-preview-doc"><For each={data().parts}>{part =>
                  <For each={part.paragraphs}>{paragraph => <p>{paragraph}</p>}</For>
                }</For></article>
              </Match>
              <Match when={data().kind === 'powerpoint'}>
                <For each={data().parts}>{part => <section class="office-preview-slide" aria-label={part.title}>
                  <h3>{part.title}</h3>
                  <Show when={part.paragraphs.length} fallback={<p class="office-preview-empty">No text on this slide.</p>}>
                    <For each={part.paragraphs}>{paragraph => <p>{paragraph}</p>}</For>
                  </Show>
                </section>}</For>
              </Match>
              <Match when={data().kind === 'excel'}>
                <Suspense fallback={<p class="office-preview-message" role="status">Opening the sheet…</p>}><SheetGrid parts={data().parts} /></Suspense>
              </Match>
            </Switch>
          </Show>
        </div>
      </>}</Show>
    </section>
  </Show>;
}
