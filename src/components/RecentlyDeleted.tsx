import { For, Show, createEffect, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { restoreWorkspace, workspaces } from '../store/workspaces';
import { studioDate } from '../studio';
import type { TrashedWorkspace } from '../generated/ipc';
import { Button, ListRow } from './ui';

/** Home → All work's Recently deleted (P1-41): the workspaces in the vault's
 *  trash, newest first, each with Restore. Lazy, so the App chunk doesn't
 *  carry it. Read again whenever the workspace list changes (a delete, a
 *  restore) into a plain signal, so a re-read never suspends Home. Hidden
 *  while the trash holds no workspace. `top`: Home with no workspace left
 *  shows it on its own, under an h2. */
export default function RecentlyDeleted(props: { top?: boolean }) {
  const [rows, setRows] = createSignal<TrashedWorkspace[]>([]);
  const [busy, setBusy] = createSignal('');
  const restore = async (t: TrashedWorkspace) => {
    setBusy(t.id);
    try { await restoreWorkspace(t.id, t.title); } finally { setBusy(''); }
  };
  createEffect(on(workspaces, () => {
    let live = true;
    onCleanup(() => { live = false; });
    api.trashedWorkspaces().then(r => { if (live) setRows(r); }, () => { if (live) setRows([]); });
  }));
  return <Show when={rows().length}>
    <section class="studio-deleted" aria-label="Recently deleted">
      <header class="home-section-head"><Show when={props.top} fallback={<h3>Recently deleted</h3>}><h2>Recently deleted</h2></Show></header>
      <div class="work-list" role="list">
        <For each={rows()}>{t => <div class="studio-work-row" role="listitem">
          <ListRow class="work-card" icon="trash" title={t.title}
            meta={`Deleted ${studioDate(t.trashedAt)}${t.blocked ? ` · ${t.blocked}` : ''}`}
            trailing={<Button size="sm" aria-label={`Restore ${t.title}`} disabled={!!t.blocked || !!busy()}
              title={t.blocked ?? 'Move it back from the vault’s trash, with its conversations'}
              onClick={() => void restore(t)}>{busy() === t.id ? 'Restoring…' : 'Restore'}</Button>}/>
        </div>}</For>
      </div>
    </section>
  </Show>;
}
