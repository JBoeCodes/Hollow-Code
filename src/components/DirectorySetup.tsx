import { createSignal, onMount, onCleanup, For, Show } from 'solid-js';
import { open } from '@tauri-apps/plugin-dialog';
import { api, type DirectoryConnection } from '../api';
import { indexedCount } from '../sourceCoverage';
import '../styles/directory-setup.css';
import { ErrorText } from './ui';
import { registerDirty } from '../settings/settingsDirty';

const DEFAULT_HOME = '~/Hollow';

/** Storeless so first-run setup never boots configured-only resources.
 *  One Hollow folder holds everything: `Projects/` for repositories and
 *  project folders, `Vault/` for Hollow's workspaces, plans, notes and
 *  outputs. Existing folders are linked in, never moved. */
export function DirectorySetup(props: { firstRun?: boolean }) {
  const [home, setHome] = createSignal(DEFAULT_HOME);
  const [existingProjects, setExistingProjects] = createSignal('');
  const [existingVault, setExistingVault] = createSignal('');
  const [bringExisting, setBringExisting] = createSignal(false);
  const [status, setStatus] = createSignal<DirectoryConnection>();
  const [busy, setBusy] = createSignal(false);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal('');
  const [saved, setSaved] = createSignal(false);
  // Typed or chosen since the last save: in Settings a section switch asks
  // before dropping it (P1-21). First run has no sections to switch.
  const [edited, setEdited] = createSignal(false);
  if (!props.firstRun) registerDirty('hollow-folder', () => edited() && !saved(), 'Hollow folder');
  let alive = true;
  onCleanup(() => { alive = false; });
  onMount(async () => {
    try {
      const s = await api.directoriesStatus();
      if (!alive) return;
      setStatus(s); setSaved(s.pendingRestart);
      if (s.homePath) setHome(s.homePath);
      else if (s.projectsPath || s.vaultPath) {
        // Two separately chosen folders from an earlier setup: offer to bring
        // them into one Hollow folder as-is.
        setExistingProjects(s.projectsPath); setExistingVault(s.vaultPath); setBringExisting(true);
      }
    } catch (e) { if (alive) setError(String(e)); } finally { if (alive) setLoading(false); }
  });
  const locked = () => busy() || saved() || loading();
  async function choose(target: 'home' | 'projects' | 'vault') {
    try {
      const title = target === 'home' ? 'Choose or create your Hollow folder' : `Choose your existing ${target === 'projects' ? 'projects' : 'vault'} folder`;
      const path = await open({ directory: true, multiple: false, title });
      if (typeof path !== 'string') return;
      ({ home: setHome, projects: setExistingProjects, vault: setExistingVault })[target](path);
      setSaved(false); setEdited(true);
    } catch (e) { setError(String(e)); }
  }
  async function connect() {
    setBusy(true); setError('');
    try {
      const result = await api.connectHollowHome(home(), bringExisting() ? existingProjects() : '', bringExisting() ? existingVault() : '');
      setStatus(result); setSaved(result.pendingRestart); setEdited(false);
      setBringExisting(false); setExistingProjects(''); setExistingVault('');
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function refresh() {
    setBusy(true); setError('');
    try { setStatus(await api.refreshDirectories()); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  const configuredHome = () => status()?.homePath ?? '';
  return <section class="directory-setup" aria-label="Hollow folder setup">
    <Show when={props.firstRun}><h2>Bring your work into Hollow</h2></Show>
    {/* In Settings the section description already says this. */}
    <Show when={props.firstRun}><p>One folder holds everything. Your repositories and project folders live in <code>Projects</code>; Hollow keeps workspaces, plans, notes and Results beside them in <code>Vault</code>.</p></Show>
    {/* Local first (next-level P18): the folder is the only required step;
        services are optional and come later, only when needed. */}
    <Show when={props.firstRun}>
      <ol class="directory-next" aria-label="Getting started">
        <li><strong>Choose the Hollow folder.</strong><span>The only step you need. No account or connector.</span></li>
        <li><strong>Start locally.</strong><span>Create a workspace, a note and a task. They are plain files in this folder and work offline.</span></li>
        <li><strong>Connect services, if you need them.</strong><span>Optional. Linear holds your projects and tasks; set it up in Settings → Sources, and connect other services in Settings → Connectors whenever you want them. Notion is available only as a read-only connector; your work stays in the vault.</span></li>
      </ol>
    </Show>
    <div class="directory-fields">
      <label>Hollow folder<span>Hollow creates it if it doesn't exist yet.</span><div><input aria-label="Hollow folder" value={home()} disabled={locked()} placeholder={DEFAULT_HOME} onInput={e => { setHome(e.currentTarget.value); setSaved(false); setEdited(true); }} /><button class="btn" disabled={locked()} onClick={() => choose('home')}>Choose…</button></div></label>
    </div>
    <Show when={configuredHome() && !bringExisting()}>
      <ul class="directory-layout" aria-label="Inside the Hollow folder">
        <li><strong>Projects</strong><span>{status()!.projectsPath}</span></li>
        <li><strong>Vault</strong><span>{status()!.vaultPath}</span></li>
      </ul>
    </Show>
    <details class="directory-existing" open={bringExisting()} onToggle={e => setBringExisting(e.currentTarget.open)}>
      <summary>Bring existing folders</summary>
      <p class="work-muted">Already keep projects or notes somewhere else? Hollow links them into the Hollow folder, so everything is in one place. Nothing is moved or copied.</p>
      <div class="directory-fields">
        <label>Existing projects folder<span>Optional. Linked in as Projects.</span><div><input aria-label="Existing projects folder" value={existingProjects()} disabled={locked()} placeholder="Leave empty to start fresh" onInput={e => { setExistingProjects(e.currentTarget.value); setEdited(true); }} /><button class="btn" disabled={locked()} onClick={() => choose('projects')}>Choose…</button></div></label>
        <label>Existing vault<span>Optional. Linked in as Vault.</span><div><input aria-label="Existing vault" value={existingVault()} disabled={locked()} placeholder="Leave empty to start fresh" onInput={e => { setExistingVault(e.currentTarget.value); setEdited(true); }} /><button class="btn" disabled={locked()} onClick={() => choose('vault')}>Choose…</button></div></label>
      </div>
    </details>
    <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
    <div class="directory-actions">
      <Show when={!saved()} fallback={<button class="btn primary" onClick={() => api.setupRestart().catch(e => setError(String(e)))}>Restart Hollow to use this folder</button>}>
        <button class="btn primary" disabled={loading() || busy() || !home().trim()} onClick={connect}>{busy() ? 'Preparing…' : configuredHome() && !bringExisting() ? 'Save' : 'Set up Hollow folder'}</button>
      </Show>
      <Show when={saved()}><button class="btn" onClick={() => setSaved(false)}>Change folder</button></Show>
      <Show when={status()?.projectsPath && !saved()}><button class="btn" disabled={busy()} onClick={refresh}>Refresh projects</button></Show>
    </div>
    <Show when={status()?.refreshedAt}><div class="directory-status" role="status"><strong>{saved() ? 'Folder ready. Restart to activate.' : 'Connected projects'}</strong><p>{status()!.projects.length} projects · {indexedCount(status()).toLocaleString('en-US')} supported files · refreshed {new Date(status()!.refreshedAt).toLocaleString()}</p><details class="settings-fold settings-row-more settings-lead-more"><summary><span class="lead-more-show">More</span><span class="lead-more-hide">Less</span></summary><p>Changes are checked every minute while Hollow is open. Ignored files, credentials, build folders and linked worktrees are excluded. Binary files require a reader or text export.</p></details><For each={status()?.warnings}>{warning => <p class="work-error">{warning}</p>}</For></div></Show>
  </section>;
}
