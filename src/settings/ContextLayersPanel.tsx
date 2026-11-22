// Settings, Sources, Context (CX17): one row per context layer, under the
// "Connect this Mac's context" card (F3, `MacContextCard.tsx`). Home (this
// Mac's name and its home file in the personal clone), Personal (the sync
// engine: status pill, last sync, Sync now, Stop syncing, Create as guided
// steps, Set it up for me (CX18, `ContextSetupCard.tsx`, a card only),
// Connect an existing repo, Local only), Organization (the read-only
// clone: folder, last pull, Check access, Request access), Project folders
// (read only), Context check (CX10: Install context check / Remove per repo)
// and Vault sync (Not set up; its first push is held). Words in
// `contextLayers.ts`, calls in `contextLayersApi.ts`; both lazy with this
// panel, so neither reaches the App chunk. Field layout reuses the task
// tracker row's classes (vault-settings.css).
import { For, Index, Show, createSignal, createUniqueId, onMount, type JSX } from 'solid-js';
import { latestRequest } from '../latestRequest';
import { Button, ErrorText } from '../components/ui';
import { showConfirm } from '../store';
import { errMsg, fmtTime, openExternal } from '../utils';
import {
  configurePersonalSync, contextCheckStatus, contextInstallCheck, contextLayersStatus, contextOrgAccessCheck, contextPersonalLocalOnly,
  contextRemoveCheck, contextSyncNow, setContextMachineSlug, setContextRoot, setContextSyncEnabled, type ContextCheckRepo,
  type ContextCheckStatus, type ContextLayersStatus, type OrgAccessCheck,
} from '../contextLayersApi';
import {
  BRIEF_COPIED, CREATE_STEPS, GUIDE_URL, NEW_REPO_URL, accessPill, blockedByRemovals, canGoLocalOnly, checkButton, checkButtonName, checkPill, homePill, leakModeLine,
  nextLoaded, orgAccessKey, orgPill, personalHasRemote, personalPill, projectsLine, remoteProblem, seedHasChanges, seedLines, slugProblem, vaultPill, type Loaded, type Pill,
} from '../contextLayers';
import type { SyncClash } from '../generated/ipc';
import { syncClashes } from '../syncClashesApi';
import { clashCountText, clashesOf } from '../syncClashes';
import { openSyncClashes } from '../store/syncClashes';
import { MacContextCard } from './MacContextCard';
import { ContextSetupCard } from './ContextSetupCard';

function Chip(props: { pill: Pill }) {
  return <span class="settings-state-chip" data-state={props.pill.state}>{props.pill.text}</span>;
}

/** "N clashes" on a synced row (CX6c): opens that root's clash cards. */
function ClashLink(props: { list: SyncClash[] | undefined; root: string; label: string }) {
  const n = () => clashesOf(props.list, props.root);
  return (
    <Show when={n() > 0}>
      <Field label="Clashes">
        <button type="button" class="btn-link" aria-label={`${clashCountText(n())} in ${props.label}: resolve`} onClick={() => openSyncClashes(props.root)}>{clashCountText(n())}</button>
      </Field>
    </Show>
  );
}

function Field(props: { label: string; children: JSX.Element }) {
  return (
    <div class="task-tracker-field">
      <span class="task-tracker-label">{props.label}</span>
      {props.children}
    </div>
  );
}

export function ContextLayersPanel() {
  // Signals, not a resource: a resource in its error state throws when read,
  // so one failed refresh took every row down (CX17 review). A failed read
  // keeps the last rows and shows its error above them.
  const [loaded, setLoaded] = createSignal<Loaded<ContextLayersStatus>>({ value: undefined, error: '' });
  const [loading, setLoading] = createSignal(true);
  const order = latestRequest();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [note, setNote] = createSignal('');
  const [remote, setRemote] = createSignal('');
  const [slug, setSlug] = createSignal('');
  const [homeFolder, setHomeFolder] = createSignal('');
  const [orgFolder, setOrgFolder] = createSignal('');
  const [access, setAccess] = createSignal<OrgAccessCheck | null>(null);
  // Set it up for me (CX18): open or closed only; nothing is saved either way.
  const [setupOpen, setSetupOpen] = createSignal(false);
  let setupButton: HTMLButtonElement | undefined;
  const setupId = createUniqueId();
  const closeSetup = () => { setSetupOpen(false); setupButton?.focus(); };
  // Context check (CX10): read with the rows. A failed read clears the rows
  // and says so (no stale state, no endless Checking…).
  const [checks, setChecks] = createSignal<ContextCheckStatus | undefined>(undefined);
  const [checksError, setChecksError] = createSignal('');
  const [clashList, setClashList] = createSignal<SyncClash[] | undefined>();

  async function refetch() {
    // The clash cards (CX6c) are a link only: a failed read shows none.
    syncClashes().then(setClashList, () => setClashList(undefined));
    const ticket = order.start();
    setLoading(true);
    let outcome: { ok: true; value: ContextLayersStatus } | { ok: false; error: unknown };
    try { outcome = { ok: true, value: await contextLayersStatus() }; } catch (error) { outcome = { ok: false, error }; }
    if (!order.isLatest(ticket)) return;
    const prev = loaded();
    const next = nextLoaded(prev, outcome, errMsg);
    // An access result is about one org folder: a changed folder clears it.
    if (orgAccessKey(prev.value) !== orgAccessKey(next.value)) setAccess(null);
    setLoaded(next);
    setLoading(false);
    try {
      const c = await contextCheckStatus();
      if (order.isLatest(ticket)) { setChecks(c); setChecksError(''); }
    } catch (e) {
      if (order.isLatest(ticket)) { setChecks(undefined); setChecksError(errMsg(e)); }
    }
  }
  onMount(() => void refetch());

  async function act(run: () => Promise<unknown>, done?: string) {
    if (busy()) return;
    setBusy(true);
    setError('');
    setNote('');
    try {
      await run();
      if (done) setNote(done);
      await refetch();
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  const s = () => loaded().value;
  const personal = () => s()?.personal;
  const remoteWhy = () => (remote().trim() ? remoteProblem(remote()) : null);

  const connect = () => void act(async () => {
    const why = remoteProblem(remote());
    if (why) throw new Error(why);
    await configurePersonalSync(remote().trim());
    setRemote('');
    setSetupOpen(false);
  }, 'Connected. The first sync runs within a minute.');
  const syncNow = () => void act(() => contextSyncNow(), 'Sync finished.');
  // The mass-removal guard's way forward: one sync past it, after a confirm.
  const syncAllowingRemovals = () => void (async () => {
    if (!await showConfirm({
      title: 'Remove those files everywhere?',
      body: 'This one sync lets the removals through: they are removed from your repo and from your other Macs. Cancel if they went missing by accident.',
      confirmLabel: 'Sync and remove',
      danger: true,
    })) return;
    await act(() => contextSyncNow(true), 'Sync finished with the removals.');
  })();
  const setSyncing = (on: boolean) => void act(() => setContextSyncEnabled(on), on ? 'Syncing again.' : 'Sync stopped. Your files stay as they are.');
  const localOnly = () => void (async () => {
    if (!await showConfirm({
      title: 'Keep personal context on this Mac only?',
      body: 'Hollow forgets the remote and stops syncing. Your files stay here and the repo stays on GitHub. Nothing here is backed up until you connect again.',
      confirmLabel: 'Local only',
    })) return;
    await act(() => contextPersonalLocalOnly(), 'Local only: sync is off and no remote is saved.');
  })();
  const saveSlug = () => void act(async () => {
    const why = slugProblem(slug());
    if (why) throw new Error(why);
    await setContextMachineSlug(slug().trim());
    setSlug('');
  }, 'Name saved.');
  const useFolder = (layer: 'personal' | 'org', path: string, clear: () => void) => void act(async () => {
    if (!path.trim()) throw new Error('Type the folder’s full path first.');
    await setContextRoot(layer, path.trim());
    clear();
  }, 'Folder saved.');
  // CX22: the same four layers without Hollow. The brief is its own lazy chunk, loaded when
  // the panel opens so the click can write the clipboard in the same tick (WebKit drops the
  // user gesture across an await); if it has not arrived yet, the click loads it first.
  let briefText = '';
  const loadBrief = () => import('../contextSetupBrief').then(m => (briefText = m.SETUP_BRIEF));
  onMount(() => void loadBrief().catch(() => undefined));
  const copyBrief = () => void act(async () => {
    await navigator.clipboard.writeText(briefText || await loadBrief());
  }, BRIEF_COPIED);
  const checkAccess = () => void act(async () => { setAccess(await contextOrgAccessCheck()); });
  // Rows are an <Index>, so a refresh updates each row in place and the
  // pressed button (disabled, so blurred, while busy) gets its focus back.
  const toggleCheck = (repo: ContextCheckRepo, button: HTMLElement) => void (async () => {
    await act(async () => {
      const done = repo.state === 'installed' ? await contextRemoveCheck(repo.path) : await contextInstallCheck(repo.path);
      setNote(done.text);
    });
    if (button.isConnected) button.focus();
  })();

  return (
    <div class="context-layers" aria-busy={busy() || loading()}>
      <Show when={loaded().error}>
        {e => <ErrorText as="p" class="settings-row-warn" text={`Couldn’t read the context layers: ${e()}`} />}
      </Show>
      <Show when={error()}><ErrorText as="p" class="settings-row-warn" text={error()} /></Show>
      <Show when={note()}><p class="settings-row-v" role="status">{note()}</p></Show>

      {/* ── Connect this Mac's context (F3): offered once per Mac pair ── */}
      <MacContextCard onChanged={() => void refetch()} />

      {/* ── Home: this Mac ── */}
      <div class="settings-row task-tracker-row" data-layer="home">
        <div class="settings-row-label">
          <div class="settings-row-k">Home</div>
          <div class="settings-row-v">Facts about this Mac, read by every chat that runs here.</div>
          <div class="task-tracker-fields">
            <Field label="Status"><Chip pill={homePill(s()?.home)} /></Field>
            <Field label="This Mac">
              <span class="task-tracker-value">{s() ? s()!.home.slug ?? 'No name yet' : 'Checking…'}</span>
              <Show when={s()?.home.file}>{f => <code class="task-tracker-value">{f()}</code>}</Show>
            </Field>
            <Field label="Name">
              <input class="input" aria-label="Name for this Mac" autocomplete="off" spellcheck={false}
                placeholder={s()?.home.slug ?? s()?.home.suggestedSlug ?? 'my-macbook'} value={slug()}
                onInput={e => setSlug(e.currentTarget.value)} disabled={busy()} />
              <Button size="sm" variant="secondary" disabled={busy() || !slug().trim()} onClick={saveSlug}>Save name</Button>
              <Show when={!s()?.home.slug && s()?.home.suggestedSlug}>
                {suggested => <Button size="sm" variant="ghost" disabled={busy()} onClick={() => setSlug(suggested())}>Use {suggested()}</Button>}
              </Show>
            </Field>
            <Field label="Clone folder">
              <code class="task-tracker-value">{s()?.home.root.path ?? ''}</code>
              <Show when={s()?.home.root.problem}>{p => <span class="task-tracker-value" data-key="unreadable">{p()}</span>}</Show>
            </Field>
            <Field label="Use a folder">
              <input class="input" aria-label="Personal clone folder" autocomplete="off" spellcheck={false}
                placeholder={s()?.home.root.path ?? ''} value={homeFolder()} onInput={e => setHomeFolder(e.currentTarget.value)} disabled={busy()} />
              <Button size="sm" variant="secondary" disabled={busy() || !homeFolder().trim()} onClick={() => useFolder('personal', homeFolder(), () => setHomeFolder(''))}>Use folder</Button>
            </Field>
          </div>
        </div>
      </div>

      {/* ── Personal: the sync engine (CX6a) ── */}
      <div class="settings-row task-tracker-row" data-layer="personal">
        <div class="settings-row-label">
          <div class="settings-row-k">Personal</div>
          <div class="settings-row-v">Your me, memory and learnings folders, synced to your own private repo.</div>
          <div class="task-tracker-fields">
            <Field label="Status">
              <Chip pill={personalPill(personal())} />
              <span class="task-tracker-value">{personal()?.lastSyncAt ? `Last sync ${fmtTime(personal()!.lastSyncAt)}` : 'Never synced'}</span>
            </Field>
            <Show when={personal()?.remote}>{r => <Field label="Repo"><code class="task-tracker-value">{r()}</code></Field>}</Show>
            <Show when={s()?.seed}>{seed => (
              <div class="task-tracker-field" data-seed-report>
                <p class="task-tracker-value" role="note">{seed().text} Nothing was changed: this is a look before the first sync.</p>
                <Show when={seedHasChanges(seed().folders)}>
                  <details class="settings-fold" data-layer-seed>
                    <summary>Files the first sync would touch</summary>
                    <ul class="task-tracker-value">
                      <For each={seedLines(seed().folders)}>{line => <li><code>{line.folder}</code> {line.what}: {line.names}</li>}</For>
                    </ul>
                  </details>
                </Show>
              </div>
            )}</Show>
            <ClashLink list={clashList()} root="personal" label="Personal" />
            <Show when={personal()?.lastError}>{e => <ErrorText as="p" class="settings-row-warn" text={e()} />}</Show>
            <Show when={personalHasRemote(personal())}>
              <div class="task-tracker-field">
                <Button size="sm" variant="secondary" disabled={busy() || !personal()?.enabled} onClick={syncNow}>Sync now</Button>
                <Show when={personal()?.enabled} fallback={<Button size="sm" variant="secondary" disabled={busy()} onClick={() => setSyncing(true)}>Turn sync on</Button>}>
                  <Button size="sm" variant="ghost" disabled={busy()} onClick={() => setSyncing(false)}>Stop syncing</Button>
                </Show>
                <Button size="sm" variant="ghost" disabled={busy() || !canGoLocalOnly(personal())} onClick={localOnly}
                  title={personal()?.state === 'syncing' ? 'Wait for the sync to finish' : undefined}>Local only</Button>
                <Show when={blockedByRemovals(personal())}>
                  <Button size="sm" variant="secondary" disabled={busy()} onClick={syncAllowingRemovals}>Sync and allow removals…</Button>
                </Show>
              </div>
            </Show>
            <Show when={!personalHasRemote(personal())}>
              <div class="task-tracker-field">
                {/* A plain button (Button's look) so focus can come back to it on Close. */}
                <button type="button" ref={setupButton} class="btn small" aria-expanded={setupOpen()} aria-controls={setupId}
                  onClick={() => (setupOpen() ? closeSetup() : setSetupOpen(true))}>Set it up for me</button>
              </div>
              <Show when={setupOpen()}>
                {/* The status carries no GitHub login (CX17), so the card shows its placeholder. */}
                <div id={setupId}>
                  <ContextSetupCard owner={null} remote={remote()} setRemote={setRemote} onClose={closeSetup} />
                </div>
              </Show>
            </Show>
            <Field label="Connect an existing repo">
              <input class="input" aria-label="Personal context repo URL" autocomplete="off" spellcheck={false}
                placeholder="git@github.com:you/personal-context.git" value={remote()}
                onInput={e => setRemote(e.currentTarget.value)} disabled={busy()} />
              <Button size="sm" variant="secondary" disabled={busy() || !remote().trim() || !!remoteWhy()} onClick={connect}>Connect</Button>
            </Field>
            <Show when={remoteWhy()}>{why => <p class="settings-row-warn">{why()}</p>}</Show>
            <details class="settings-fold" data-layer-create="personal">
              <summary>Create a private repo</summary>
              <ol class="task-tracker-value">
                <For each={CREATE_STEPS}>{step => <li>{step}</li>}</For>
              </ol>
              <Button size="sm" variant="secondary" onClick={() => openExternal(NEW_REPO_URL)}>Open GitHub</Button>
            </details>
            <Show when={!personalHasRemote(personal())}>
              <p class="task-tracker-value">Local only until you connect a repo: your files stay on this Mac.</p>
            </Show>
          </div>
        </div>
      </div>

      {/* ── Organization: the read-only clone (CX5) ── */}
      <div class="settings-row task-tracker-row" data-layer="org">
        <div class="settings-row-label">
          <div class="settings-row-k">Organization</div>
          <div class="settings-row-v">Shared company context, read only on every Mac. Hollow never creates it.</div>
          <div class="task-tracker-fields">
            <Field label="Status">
              <Chip pill={orgPill(s()?.org.root)} />
              <span class="task-tracker-value">{s()?.org.lastPullAt ? `Last pull ${fmtTime(s()!.org.lastPullAt)}` : 'Not pulled yet'}</span>
            </Field>
            <Show when={s()?.org.remote}>{r => <Field label="Repo"><code class="task-tracker-value">{r()}</code></Field>}</Show>
            <Field label="Clone folder">
              <code class="task-tracker-value">{s()?.org.root.path ?? ''}</code>
              <Show when={s()?.org.root.problem}>{p => <span class="task-tracker-value" data-key="unreadable">{p()}</span>}</Show>
            </Field>
            <Field label="Use a folder">
              <input class="input" aria-label="Org clone folder" autocomplete="off" spellcheck={false}
                placeholder={s()?.org.root.path ?? ''} value={orgFolder()} onInput={e => setOrgFolder(e.currentTarget.value)} disabled={busy()} />
              <Button size="sm" variant="secondary" disabled={busy() || !orgFolder().trim()} onClick={() => useFolder('org', orgFolder(), () => setOrgFolder(''))}>Use folder</Button>
            </Field>
            <Field label="Access">
              <Button size="sm" variant="secondary" disabled={busy() || !s()?.org.root.configured} onClick={checkAccess}>Check access</Button>
              <Show when={access()}>{a => <><Chip pill={accessPill(a())} /><span class="task-tracker-value">{a().text}</span></>}</Show>
            </Field>
            <p class="task-tracker-value">Request access: ask the org repo’s owner to add your GitHub account, then Check access.</p>
            <Field label="Set up without Hollow">
              <Button size="sm" variant="secondary" onClick={() => openExternal(GUIDE_URL)}>Open the guide</Button>
              <Button size="sm" variant="ghost" disabled={busy()} aria-label="Copy the setup brief for another AI assistant" onClick={copyBrief}>Copy setup brief</Button>
              <span class="task-tracker-value">For a colleague, or a tool, that does not use Hollow.</span>
            </Field>
          </div>
        </div>
      </div>

      {/* ── Project folders: read only for now ── */}
      <div class="settings-row task-tracker-row" data-layer="projects">
        <div class="settings-row-label">
          <div class="settings-row-k">Project folders</div>
          <div class="settings-row-v">Each project’s own CLAUDE.md or AGENTS.md. Agents read them in place.</div>
          <div class="task-tracker-fields">
            <p class="task-tracker-value">{projectsLine(s()?.projects)}</p>
            <For each={s()?.projects.withContext ?? []}>
              {p => (
                <Field label={p.title}>
                  <span class="task-tracker-value">{p.files.join(', ')}{p.git ? '' : ' (not a git repo)'}</span>
                </Field>
              )}
            </For>
          </div>
        </div>
      </div>

      {/* ── Context check (CX10): a local pre-push hook per repo ── */}
      <div class="settings-row task-tracker-row" data-layer="checks">
        <div class="settings-row-label">
          <div class="settings-row-k">Context check</div>
          <div class="settings-row-v">A local pre-push hook that stops a push carrying a secret. Never committed.</div>
          <div class="task-tracker-fields">
            <p class="task-tracker-value">{leakModeLine(checks(), checksError())}</p>
            <Index each={checks()?.repos ?? []}>
              {repo => (
                <Field label={repo().title}>
                  <Chip pill={checkPill(repo())} />
                  <Show when={checkButton(repo())}>
                    {label => <Button size="sm" variant={label() === 'Remove' ? 'ghost' : 'secondary'} aria-label={checkButtonName(label(), repo().title)}
                      disabled={busy()} onClick={e => toggleCheck(repo(), e.currentTarget)}>{label()}</Button>}
                  </Show>
                  <Show when={repo().problem}>{p => <span class="task-tracker-value">{p()}</span>}</Show>
                </Field>
              )}
            </Index>
          </div>
        </div>
      </div>

      {/* ── Vault: CX6b builds its root; the first push is held ── */}
      <div class="settings-row task-tracker-row" data-layer="vault">
        <div class="settings-row-label">
          <div class="settings-row-k">Vault sync</div>
          <div class="settings-row-v">Notes, workspaces and outputs. Stays on this Mac for now.</div>
          <div class="task-tracker-fields">
            <Field label="Status"><Chip pill={vaultPill(s()?.vault)} /></Field>
            <ClashLink list={clashList()} root="vault" label="Vault sync" />
            <Show when={s()?.vault.remote}>{r => <Field label="Repo"><code class="task-tracker-value">{r()}</code></Field>}</Show>
            <Field label="Connect">
              <Button size="sm" variant="secondary" disabled title="The vault’s first push is held">Connect</Button>
              <span class="task-tracker-value">Held: the vault’s first push waits for its own review.</span>
            </Field>
          </div>
        </div>
      </div>
    </div>
  );
}
