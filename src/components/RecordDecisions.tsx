import { For, Show, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import { decisionLabel, decisionScope, libraryDecisionRows } from '../decisionRows';
import { openVaultNote } from '../store';
import { workspaces } from '../store/workspaces';
import { errMsg } from '../utils';
import { ErrorText } from './ui';

/**
 * Library, Records' Decisions (COO rebuild L4, 2026-11-10; master plan #20's
 * list was on a workspace's Overview, gone with the workspace panel): every
 * recorded decision that isn't superseded, with where it applies and its
 * state. Revisit and Settle again are the person's call here (agents set it
 * with the `records.records_set_decision_status` action, which asks you on
 * a card first); superseding happens on a
 * card, never here.
 */
export function RecordDecisions() {
  const [error, setError] = createSignal('');
  const [list, { refetch }] = createResource(() => api.recordsDecisions().catch(e => { setError(errMsg(e)); return []; }));
  const rows = () => libraryDecisionRows(list.latest ?? []);
  const titleOf = (id: string) => workspaces().find(w => w.id === id)?.title;
  async function set(path: string, status: 'settled' | 'revisit') {
    try { setError(''); await api.recordsSetDecisionStatus(path, status); await refetch(); } catch (e) { setError(errMsg(e)); }
  }
  return (
    <Show when={rows().length || error()}>
      <section class="library-decisions" aria-label="Decisions">
        <Show when={error()}>{msg => <ErrorText class="library-decisions-error" text={msg()}/>}</Show>
        <Show when={rows().length}>
          <details class="library-decisions-list"><summary>Decisions <small>{rows().length} · {rows().filter(d => d.status === 'revisit').length} to revisit</small></summary>
            <ul><For each={rows()}>{d =>
              <li classList={{ revisit: d.status === 'revisit' }}>
                <button type="button" class="btn-link" onClick={() => openVaultNote(d.path)}>{d.text}</button>
                <small>{decisionScope(d.scope, titleOf)} · {decisionLabel(d)}</small>
                <button type="button" class="btn small ghost" onClick={() => void set(d.path, d.status === 'revisit' ? 'settled' : 'revisit')}>{d.status === 'revisit' ? 'Settle again' : 'Revisit'}</button>
              </li>
            }</For></ul>
          </details>
        </Show>
      </section>
    </Show>
  );
}
