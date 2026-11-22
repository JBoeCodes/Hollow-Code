import { focusWorkspaceConversation, openWorkspaceTitle } from '../store/workspaces';
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { MarkdownView } from '../components/MarkdownView';
import { Icon } from '../icons';
import { api } from '../api';
import {
  skillsList, refetchSkills,
  setActivePage, setChatInput, flashFooter, chatBusy,
  openContextMenu, type ContextMenuItem,
  enableSkillsResource, showConfirm,
  skillOpenRequest, setSkillOpenRequest,
} from '../store';
import type { Skill, SkillFileChanges, SkillImportClash, SkillImportPreview, SkillImportSource, SkillProvenance } from '../types';
import { errMsg, stripFrontmatter } from '../utils';
import { isBrowserMockMode } from '../mockMode';
import { SKILL_IMPORT_EXTENSIONS, captureDrop, fileSource, formatBytes, isFileDrag, suggestedRename } from '../skillImport';
import { effectLabel, findingsOf, rulesInOrder, statusTag, tableSummary, versionLine } from '../skillPortability';
import { buildCatalog, catalogCounts, entryLint, filterCatalog, lintBadge, lintFindingsInOrder, precedenceNote, skillAvailability, skillOwner, skillRuntime, useSkillPrompt, type CatalogEntry, type CatalogFilter } from '../skillCatalog';
import { AGENT_LABELS, KNOWN_AGENTS, chatAgent, type ChatAgent } from '../store/runtime';
import { Segmented, ErrorText } from '../components/ui';
import { RefreshNotice } from '../components/RefreshNotice';
import { WaitingLink } from '../components/WaitingLink';
import { latestRequest } from '../latestRequest';
import { createSkillWithChoice } from '../store/writeSkill';
import '../styles/skills.css';

type FilterKind = CatalogFilter;

/// Reusable capabilities managed in Settings. Choosing a skill returns to
/// the current workspace; it never launches a separate global conversation.
export function SkillsView(props: { headerActions?: JSX.Element }) {
  onMount(enableSkillsResource);
  const [filter, setFilter] = createSignal('');
  const [kind, setKind] = createSignal<FilterKind>('all');
  const [active, setActive] = createSignal<Skill | null>(null);
  const [creating, setCreating] = createSignal(false);
  const [createBusy, setCreateBusy] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  const [newName, setNewName] = createSignal('');
  const [newDesc, setNewDesc] = createSignal('');
  const [newBody, setNewBody] = createSignal('You are a [persona]. When asked to X, do Y.\n');
  // P1-57: the form also edits a writable skill (`editing`), and its errors
  // stay in it until dismissed; a delete's error stays in the detail pane.
  const [editing, setEditing] = createSignal<Skill | null>(null);
  const [extraHeader, setExtraHeader] = createSignal<string[]>([]);
  const edits = latestRequest();
  let bodyInput: HTMLTextAreaElement | undefined;
  const [formError, setFormError] = createSignal<string | null>(null);
  const [actionError, setActionError] = createSignal<string | null>(null);
  // What Use does, named: the open workspace's conversation, or New workspace
  // when none is open (focusWorkspaceConversation).
  const useLabel = () => { const t = openWorkspaceTitle(); return t ? `Use in ${t}` : 'Use in a new workspace…'; };
  // Import: the panel, a drag over the page, the staged preview (Rust holds
  // its files until Install or Cancel), what is running and what failed.
  const [importing, setImporting] = createSignal(false);
  const [dropping, setDropping] = createSignal(false);
  const [preview, setPreview] = createSignal<SkillImportPreview | null>(null);
  const [importBusy, setImportBusy] = createSignal<string | null>(null);
  const [importError, setImportError] = createSignal<string | null>(null);
  const [clash, setClash] = createSignal<SkillImportClash>('cancel');
  // A check still running when the panel's import is dropped discards what it
  // staged instead of showing it (X21).
  const imports = latestRequest();
  // Nested dragenter/dragleave pairs, so the highlight neither flickers nor sticks.
  let dragDepth = 0;
  const installing = () => importBusy() === 'Installing…';
  onCleanup(() => { imports.cancel(); const p = preview(); if (p) void api.discardSkillImport(p.token).catch(() => {}); });
  onMount(() => {
    const reset = () => { dragDepth = 0; setDropping(false); };
    window.addEventListener('dragend', reset);
    window.addEventListener('drop', reset);
    window.addEventListener('blur', reset);
    onCleanup(() => { window.removeEventListener('dragend', reset); window.removeEventListener('drop', reset); window.removeEventListener('blur', reset); });
  });
  // Availability is per agent (next-level P18): the conversation's agent by
  // default, switchable to see what another agent would get.
  const [agent, setAgent] = createSignal<ChatAgent>(chatAgent());
  const agentLabel = () => AGENT_LABELS[agent()] ?? agent();

  // Cross-page deep-link (palette skill rows): select the named skill once
  // the list has it. Left pending while the resource is still loading so a
  // request placed before the fetch resolves still lands.
  createEffect(() => {
    const req = skillOpenRequest();
    if (!req) return;
    const match = (skillsList() ?? []).find(s => s.name === req.name);
    if (!match) return;
    closeForm();
    void closeImport();
    setActive(match);
    setSkillOpenRequest(null);
  });

  // One entry per name; duplicates become copies with ownership and
  // precedence in the detail pane (audit H13).
  const catalog = createMemo(() => buildCatalog(skillsList() ?? []));
  const counts = createMemo(() => catalogCounts(catalog()));
  const shown = createMemo(() => filterCatalog(catalog(), filter(), kind()));
  const entry = createMemo(() => { const a = active(); return a ? catalog().find(e => e.name === a.name) ?? null : null; });
  // A failed read is its own error, shown in plain words above the
  // instructions, never fed into the Markdown body (overnight A09).
  const [content] = createResource(active, async (s): Promise<{ text: string; error: string | null }> => {
    if (!s) return { text: '', error: null };
    try { return { text: await api.readSkill(s.path), error: null }; } catch (e) { return { text: '', error: errMsg(e) }; }
  });

  async function useSkill(s: Skill) {
    if (chatBusy()) { flashFooter('Finish or stop the current turn before choosing a skill.'); return; }
    if (!await focusWorkspaceConversation()) return;
    setChatInput(draft=>`${draft}${draft?'\n\n':''}${useSkillPrompt(s)}`);
  }

  function skillMenu(e: MouseEvent, s: Skill) {
    const items: ContextMenuItem[] = [
      { label: useLabel(), action: () => useSkill(s) },
      { label: 'Open',        action: () => { setActive(s); } },
      ...(s.writable ? [{ label: 'Edit', action: () => void openEdit(s) }] : []),
      { separator: true },
      { label: 'Delete', action: () => deleteUser(s), danger: true, disabled: !s.writable },
    ];
    openContextMenu(e, items);
  }

  async function deleteUser(s: Skill) {
    if (!s.writable) return;
    const ok = await showConfirm({
      title: `Delete user skill "${s.name}"?`,
      body: 'Removes this writable skill from Hollow’s local skill library.',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    setActionError(null);
    try {
      await api.deleteUserSkill(s.name);
      if (active()?.path === s.path) setActive(null);
      await refetchSkills();
      flashFooter('Skill deleted', 1500);
    } catch (e) {
      setActive(s);
      setActionError(`Couldn’t delete this skill: ${errMsg(e)}`);
    }
  }

  function openCreate() {
    void closeImport();
    edits.cancel();
    setActionError(null);
    setEditing(null);
    setExtraHeader([]);
    setFormError(null);
    setNewName('');
    setNewDesc('');
    setNewBody('You are a [persona]. When asked to X, do Y.\n');
    setCreating(true);
  }

  /** Edit a writable skill: the same form, prefilled from its SKILL.md, saved
   *  in replace mode over that one folder (P0-12's guard: Rust lists a skill
   *  as writable only when its folder in ~/.agents/skills has its name). */
  async function openEdit(s: Skill) {
    if (!s.writable) return;
    // A read still running when something else opens (New skill, Import,
    // another row) is dropped, so it can't replace that form.
    const ticket = edits.start();
    setActionError(null);
    let text: string;
    try { text = await api.readSkill(s.path); }
    catch (e) { if (!disposed && edits.isLatest(ticket)) { setActive(s); setActionError(`Couldn’t open this skill to edit: ${errMsg(e)}`); } return; }
    if (disposed || !edits.isLatest(ticket)) return;
    void closeImport();
    const header = /^---\r?\n([\s\S]*?)\r?\n---/.exec(text)?.[1] ?? '';
    setExtraHeader([...header.matchAll(/^([A-Za-z_][\w-]*):/gm)].map(m => m[1]).filter(k => k !== 'name' && k !== 'description'));
    setFormError(null);
    setEditing(s);
    setNewName(s.name);
    setNewDesc(s.description);
    setNewBody(stripFrontmatter(text).replace(/^\s*\n/, '').trimEnd());
    setCreating(true);
    queueMicrotask(() => bodyInput?.focus());
  }

  function closeForm() { edits.cancel(); setCreating(false); setEditing(null); setFormError(null); setActionError(null); }

  async function submitCreate(e: Event) {
    e.preventDefault();
    const name = newName().trim();
    if (!name || createBusy()) return;
    const description = newDesc(), body = newBody(), edited = editing();
    // An error belongs to this form while it is open; a result only while
    // the form still says what was saved.
    const open = () => !disposed && creating() && editing() === edited;
    const current = () => open() && newName().trim() === name && newDesc() === description && newBody() === body;
    setCreateBusy(true);
    setFormError(null);
    try {
      const skill = edited
        ? await api.writeUserSkill(edited.name, description, body, false)
        : await createSkillWithChoice(name, description, body, current, () => {
          if (open()) setFormError(`Nothing was saved: /${name} already exists. Choose another name, or create it again and choose Replace.`);
        });
      if (!skill || !current()) return;
      await refetchSkills();
      if (!current()) return;
      setActive((skillsList() ?? []).find(s => s.path === skill.path) ?? skill);
      closeForm();
      flashFooter(edited ? `Skill ${skill.name} saved` : `Skill ${skill.name} added`, 1500);
    } catch (err) {
      if (open()) setFormError(`Couldn’t save this skill: ${errMsg(err)}`);
    } finally { if (!disposed) setCreateBusy(false); }
  }

  function openImport() {
    closeForm();
    setImportError(null);
    setImporting(true);
  }

  /** Drop the staged copy, if any (Rust deletes its files). */
  async function dropStaged() {
    imports.cancel();
    const p = preview();
    setPreview(null);
    setClash('cancel');
    if (p) await api.discardSkillImport(p.token).catch(() => {});
  }

  async function closeImport() {
    // Install claims its staged copy in Rust; leaving now would race it.
    if (!importing() || installing()) return;
    setImporting(false);
    setImportError(null);
    setImportBusy(null);
    await dropStaged();
  }

  // SD2: the source of the last refused import, so its suggested name can
  // stage the same skill again, renamed.
  let lastSource: (() => Promise<SkillImportSource>) | null = null;

  async function stageImport(source: () => Promise<SkillImportSource>, renameTo?: string) {
    if (importBusy()) return;
    await dropStaged();
    lastSource = source;
    const gen = imports.peek();
    setImportError(null);
    setImportBusy('Checking the skill…');
    try {
      const staged = await api.previewSkillImport(await source(), renameTo);
      if (imports.isLatest(gen)) setPreview(staged);
      else void api.discardSkillImport(staged.token).catch(() => {});
    } catch (err) {
      if (imports.isLatest(gen)) setImportError(errMsg(err));
    } finally {
      if (imports.isLatest(gen)) setImportBusy(null);
    }
  }

  function importRenamed(name: string) {
    if (lastSource) void stageImport(lastSource, name);
  }

  async function pickImport(directory: boolean) {
    let path: string | null;
    if (isBrowserMockMode()) {
      path = localStorage.getItem('mockSkillImportPath') ?? '/mock/Downloads/pdf-tools.skill';
    } else {
      const picked = await openDialog(directory
        ? { multiple: false, directory: true, title: 'Import a skill folder' }
        : { multiple: false, directory: false, title: 'Import a skill', filters: [{ name: 'Skill (.skill, .zip, SKILL.md)', extensions: SKILL_IMPORT_EXTENSIONS }] });
      path = typeof picked === 'string' ? picked : null;
    }
    if (path) await stageImport(async () => ({ kind: 'path', path: path! }));
  }

  // Repo ideas 10-27 #2: Check for update… stages the recorded source on
  // Import's own card, with Replace chosen; nothing changes until Install.
  async function checkUpdate(name: string) {
    if (importBusy() || installing()) return;
    closeForm();
    await dropStaged();
    const gen = imports.peek();
    setImportError(null);
    setImporting(true);
    setImportBusy('Checking for an update…');
    try {
      const staged = await api.stageSkillUpdate(name);
      if (imports.isLatest(gen)) { setPreview(staged); setClash('replace'); }
      else void api.discardSkillImport(staged.token).catch(() => {});
    } catch (err) {
      if (imports.isLatest(gen)) setImportError(errMsg(err));
    } finally {
      if (imports.isLatest(gen)) setImportBusy(null);
    }
  }

  async function installImport() {
    const p = preview();
    if (!p || importBusy() || (p.existing && clash() === 'cancel')) return;
    setImportBusy('Installing…');
    setImportError(null);
    try {
      const out = await api.commitSkillImport(p.token, p.existing ? clash() : 'cancel');
      setPreview(null);
      setImporting(false);
      bumpProvenance(n => n + 1);
      await refetchSkills();
      setActive((skillsList() ?? []).find(s => s.path === out.skill.path) ?? out.skill);
      flashFooter(out.renamedFrom ? `Skill ${out.skill.name} added (kept beside ${out.renamedFrom})` : out.replaced ? `Skill ${out.skill.name} replaced` : `Skill ${out.skill.name} added`, 2500);
    } catch (err) {
      const text = errMsg(err);
      // The name was taken after the preview: offer the clash choice (Rust
      // checks again, and refuses to replace a link).
      if (!p.existing && text.includes('already exists')) setPreview({ ...p, existing: `~/.agents/skills/${p.name}/SKILL.md`, canReplace: true });
      setImportError(text);
    } finally {
      setImportBusy(null);
    }
  }

  // A file or folder dropped anywhere on the page imports it. Conversation
  // and task drags carry no files, so they pass through.
  function onDragEnter(e: DragEvent) {
    if (!isFileDrag(e.dataTransfer)) return;
    dragDepth++;
    setDropping(true);
  }
  function onDragOver(e: DragEvent) {
    if (!isFileDrag(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  }
  function onDragLeave(e: DragEvent) {
    if (!isFileDrag(e.dataTransfer)) return;
    dragDepth = Math.max(0, dragDepth - 1);
    if (dragDepth === 0) setDropping(false);
  }
  function onDrop(e: DragEvent) {
    if (!isFileDrag(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth = 0;
    setDropping(false);
    if (creating()) { flashFooter('Finish or cancel the new skill before importing one.', 3000); return; }
    if (installing()) return;
    // Read the drop now: the DataTransfer empties once this handler returns.
    const capture = captureDrop(e.dataTransfer);
    if (!capture) return;
    openImport();
    if ('error' in capture) { setImportError(capture.error); return; }
    void stageImport(() => fileSource(capture.file));
  }

  return (
    <div class="skills-catalog" classList={{ dropping: dropping() }} onDragEnter={onDragEnter} onDragOver={onDragOver} onDragLeave={onDragLeave} onDrop={onDrop}>
      <div class="skills-toolbar">
        <label class="skills-search"><Icon name="search" size={14} /><input type="search" aria-label="Find a skill" placeholder="Find a skill…" value={filter()} onInput={e => setFilter(e.currentTarget.value)} /></label>
        <div class="ui-segmented skills-filter" role="group" aria-label="Show skills from">
          <For each={[['all', 'All'], ['shared', 'Shared'], ['provider', 'Providers'], ['plugin', 'Plugins'], ['marketplace', 'Marketplace']] as [FilterKind, string][]}>{([k, label]) =>
            <Show when={k === 'all' || counts()[k] > 0}><button type="button" aria-pressed={kind() === k} onClick={() => setKind(k)}>{label} <span class="skills-count">{counts()[k]}</span></button></Show>
          }</For>
        </div>
        <div class="skills-agent" data-settings-control="Available to"><span class="skills-agent-k">Available to</span>
          <Segmented label="Available to" value={agent()} onChange={setAgent} items={KNOWN_AGENTS.map(a => ({ value: a, label: AGENT_LABELS[a] }))} />
          <Show when={agent() === chatAgent()}><span class="skills-agent-note">this conversation’s agent</span></Show>
        </div>
        <span class="skills-total">{(skillsList() ?? []).length} installed · {catalog().length} names</span>
        {props.headerActions}
        <button class="btn small" onClick={openImport}><Icon name="upload" size={14} /> Import skill</button>
        <button class="btn primary small" onClick={openCreate}><Icon name="plus" size={14} /> New skill</button>
      </div>
      <RefreshNotice resource={skillsList} onRetry={refetchSkills} what="skills" />
      {/* NV7: chats that missed a skill, waiting as eval cases. */}
      <WaitingLink context="evals" hideWhenEmpty />
      <div class="skills-panes">
        <div class="skills-list" role="list" aria-label="Skills" onKeyDown={e => {
          if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
          const rows = [...e.currentTarget.querySelectorAll<HTMLButtonElement>('.skills-row')];
          const at = rows.indexOf(document.activeElement as HTMLButtonElement);
          const next = rows[Math.max(0, Math.min(rows.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))];
          if (next) { e.preventDefault(); next.focus(); }
        }}>
          <For each={shown()} fallback={<p class="skills-empty">{skillsList.loading ? 'Loading skills…' : 'No skill matches. Try another word or show all sources.'}</p>}>{e =>
            <div role="listitem"><button type="button" class="skills-row" classList={{ active: entry()?.name === e.name }} aria-current={entry()?.name === e.name ? 'true' : undefined}
              onClick={() => { closeForm(); setActionError(null); void closeImport(); setActive(e.copies[0]); }} onContextMenu={ev => skillMenu(ev, e.copies[0])}>
              <span class="skills-row-top"><strong>{e.name}</strong><span class="skills-badges"><Show when={entryLint(e)}>{l => <span class="skills-badge lint" data-grade={lintBadge(l()).grade} title={lintBadge(l()).title}>{lintBadge(l()).label}</span>}</Show><For each={e.copies}>{c => <span class={`skills-badge ${skillOwner(c).kind}`}>{skillOwner(c).badge}</span>}</For></span></span>
              <span class="skills-avail" data-auto={skillAvailability(e, agent(), agentLabel()).auto ? 'true' : 'false'}>{skillAvailability(e, agent(), agentLabel()).auto ? `${agentLabel()} may load it` : 'On request'}</span>
              <Show when={e.description}><span class="skills-row-desc">{e.description}</span></Show>
            </button></div>
          }</For>
        </div>
        <section class="skills-detail" aria-label="Skill details">
          <Show when={creating()}>
            <div class="skill-form">
              <h2>{editing() ? 'Edit skill' : 'New skill'}</h2>
              <Show when={editing()} fallback={<p class="work-muted">Saved to Hollow’s shared skill library (~/.agents/skills). No agent loads it by itself: a chat lists it to the agent when a request fits, and the skill’s Use in button asks the agent to read it.</p>}>
                <p class="work-muted">Saves over this skill’s SKILL.md in Hollow’s shared skill library (~/.agents/skills). Its name stays; its other files are left alone.</p>
                <Show when={extraHeader().length}><p class="skill-form-note" role="note">
                  <Show when={extraHeader().length}>Saving keeps only the name and description in this skill’s header, so {extraHeader().join(', ')} will go. </Show>
                </p></Show>
              </Show>
              <form onSubmit={submitCreate}>
                <label><span class="form-label">Name</span><input autofocus={!editing()} readOnly={!!editing()} placeholder="my-skill" value={newName()} onInput={e => setNewName(e.currentTarget.value)} pattern="[a-zA-Z0-9_-]+" /><small>{editing() ? 'A skill is renamed by creating it again under the new name.' : 'Letters, numbers, dashes and underscores.'}</small></label>
                <label><span class="form-label">When to use it</span><textarea class="skill-description-input" rows={3} placeholder="When should an agent use this skill?" value={newDesc()} onInput={e => setNewDesc(e.currentTarget.value)} /></label>
                <label><span class="form-label">Instructions (Markdown)</span><textarea ref={bodyInput} rows={12} value={newBody()} onInput={e => setNewBody(e.currentTarget.value)} /></label>
                <Show when={formError()}>{err => <div class="skill-error-row"><ErrorText as="div" class="skill-import-error skill-form-error" text={err()} /><button type="button" class="btn ghost small" onClick={() => setFormError(null)}>Dismiss</button></div>}</Show>
                <div class="skill-form-actions"><button type="submit" class="btn primary" disabled={!newName().trim() || createBusy()}>{editing() ? 'Save changes' : 'Create skill'}</button><button type="button" class="btn ghost" onClick={closeForm}>Cancel</button></div>
              </form>
            </div>
          </Show>
          <Show when={importing()}>
            <SkillImportPanel preview={preview()} busy={importBusy()} error={importError()} clash={clash()} onClash={setClash} onRename={importRenamed}
              onPickFile={() => void pickImport(false)} onPickFolder={() => void pickImport(true)} onInstall={() => void installImport()} onCancel={() => void closeImport()} />
          </Show>
          <Show when={!creating() && !importing() && entry()}>{e => <>
            <Show when={actionError()}>{err => <div class="skill-error-row"><ErrorText as="div" class="skill-import-error skill-action-error" text={err()} /><button type="button" class="btn ghost small" onClick={() => setActionError(null)}>Dismiss</button></div>}</Show>
            <SkillDetail entry={e()} selected={active()!} content={content()?.text ?? ''} readError={content()?.error ?? null} agent={agent()} agentLabel={agentLabel()} useLabel={useLabel()} onSelect={c => { setActionError(null); setActive(c); }} onUse={useSkill} onEdit={s => void openEdit(s)} onDelete={deleteUser} onCheckUpdate={name => void checkUpdate(name)} />
          </>}</Show>
          <Show when={!creating() && !importing() && !entry()}>
            <div class="skills-intro">
              <Icon name="zap" size={22} />
              <h2>Reusable instructions for your agents.</h2>
              <p>Choose a skill to see what it does, who owns each copy and which agents can use it. Hollow doesn’t load skills into agents: a chat lists the shared skills that fit its request, which the agent reads whole when it needs one, and a skill’s Use in button asks the agent to read the one you choose (its text goes with the message when it is small). A provider’s own runtime may also load the skills installed for it.</p>
            </div>
          </Show>
        </section>
      </div>
    </div>
  );
}

/// Import skill: a drop target and pickers, then the staged skill's name,
/// files and scripts, and the name-clash choice, before Install.
function SkillImportPanel(props: {
  preview: SkillImportPreview | null;
  busy: string | null;
  error: string | null;
  clash: SkillImportClash;
  onClash: (c: SkillImportClash) => void;
  onRename: (name: string) => void;
  onPickFile: () => void;
  onPickFolder: () => void;
  onInstall: () => void;
  onCancel: () => void;
}) {
  return (
    <div class="skill-form skill-import">
      <h2>{props.preview?.update ? 'Update skill' : 'Import skill'}</h2>
      <Show when={props.preview} fallback={
        <>
          <p class="work-muted">Installs a .skill export, a .zip, a skill folder or a SKILL.md into Hollow’s shared skill library (~/.agents/skills), like New skill. You see what’s inside before anything is installed.</p>
          <div class="skill-drop" aria-busy={props.busy ? 'true' : 'false'}>
            <Icon name="upload" size={22} />
            <p><strong>Drop a skill anywhere on this page</strong></p>
            <p class="skill-drop-kinds">A .skill export, a .zip or a SKILL.md; for a skill folder, use Choose folder…</p>
            <div class="skill-form-actions">
              <button type="button" class="btn primary" disabled={!!props.busy} onClick={props.onPickFile}>Choose file…</button>
              <button type="button" class="btn" disabled={!!props.busy} onClick={props.onPickFolder}>Choose folder…</button>
              <button type="button" class="btn ghost" onClick={props.onCancel}>Cancel</button>
            </div>
          </div>
          <Show when={props.busy}><p class="work-muted" role="status">{props.busy}</p></Show>
        </>
      }>{p => {
        const scripts = () => p().files.filter(f => f.script).map(f => f.path);
        const target = () => p().existing && props.clash === 'keep-both' ? p().keepBothName : p().name;
        return (
          <div class="skill-import-preview">
            <h3 class="skill-import-name">/{p().name}</h3>
            <p class="skill-detail-desc">{p().description}</p>
            <p class="skill-import-meta">{p().files.length} {p().files.length === 1 ? 'file' : 'files'} · {formatBytes(p().totalBytes)}{p().folder ? ` · from the folder ${p().folder}` : ''} · installs to ~/.agents/skills/{target()}</p>
            <Show when={p().update}>{u => {
              const c = () => u().changes;
              const count = () => c().added.length + c().modified.length + c().removed.length;
              return (
                <div class="skill-update-diff" role="note">
                  <Show when={count()} fallback={<p><strong>Nothing changed</strong> since the installed copy.</p>}>
                    <p><strong>Changes from the installed copy:</strong> {[c().modified.length && `${c().modified.length} changed`, c().added.length && `${c().added.length} added`, c().removed.length && `${c().removed.length} removed`].filter(Boolean).join(', ')}</p>
                    <ul class="skill-update-files">
                      <For each={c().modified}>{f => <li><span class="skills-badge">changed</span> {f}</li>}</For>
                      <For each={c().added}>{f => <li><span class="skills-badge">added</span> {f}</li>}</For>
                      <For each={c().removed}>{f => <li><span class="skills-badge">removed</span> {f}</li>}</For>
                    </ul>
                  </Show>
                  <Show when={u().scriptsChanged.length}><p class="skill-update-scripts"><Icon name="alert" size={14} /> Scripts added or changed: {u().scriptsChanged.join(', ')}. Read them below before you install.</p></Show>
                  {/* Each changed or added file's text (review of #1050): a
                      script opens by itself; binary or large files are listed. */}
                  <For each={u().diffs}>{d => (
                    <details class="skill-update-file" open={d.script}>
                      <summary><code>{d.path}</code>{d.script ? ' · script' : ''}{d.listedOnly ? ' · binary or over 256 KB, not shown' : d.truncated ? ' · shortened' : ''}</summary>
                      <Show when={!d.listedOnly}><pre class="skill-update-text">{d.text}</pre></Show>
                    </details>
                  )}</For>
                </div>
              );
            }}</Show>
            <Show when={p().renamedFrom}>
              <p class="skill-import-renamed" role="note">Renamed from <code>{p().renamedFrom}</code> because that name has characters a skill name can’t use.{(p().rewritten ?? []).length ? ` References to the old name were rewritten in ${(p().rewritten ?? []).map(r => `${r.path} (${r.count})`).join(', ')}.` : ' No file referred to the old name.'}</p>
            </Show>
            <Show when={p().hasScripts}>
              <div class="skill-import-warning" role="note">
                <Icon name="alert" size={16} />
                <div><strong>This skill contains scripts.</strong> They run as code on this Mac, with your permissions, when an agent follows the skill. Install it only if you trust where it came from.
                  <span class="skill-import-scripts">{scripts().join(', ')}</span></div>
              </div>
            </Show>
            <Show when={p().existing}>
              <fieldset class="skill-import-clash">
                <legend>A shared skill named {p().name} already exists.</legend>
                <label><input type="radio" name="skill-import-clash" checked={props.clash === 'replace'} disabled={!p().canReplace} onChange={() => props.onClash('replace')} /> Replace it
                  <small>{p().canReplace ? 'Its folder is swapped for this one; its old files go.' : 'It is a link, so Hollow won’t replace it.'}</small></label>
                <label><input type="radio" name="skill-import-clash" checked={props.clash === 'keep-both'} onChange={() => props.onClash('keep-both')} /> Keep both
                  <small>Installs this one as {p().keepBothName}.</small></label>
              </fieldset>
            </Show>
            <h3 class="skill-subhead">Files</h3>
            <table class="skill-import-files">
              <tbody><For each={p().files}>{f =>
                <tr><td class="skill-import-path">{f.path}<Show when={f.script}> <span class="skills-badge script">script</span></Show></td><td class="skill-import-size">{formatBytes(f.size)}</td></tr>
              }</For></tbody>
            </table>
            <Show when={p().skipped.length}>
              <p class="work-muted skill-import-skipped">Left out ({p().skipped.length}): {p().skipped.slice(0, 5).join(', ')}{p().skipped.length > 5 ? ', …' : ''}</p>
            </Show>
            <div class="skill-form-actions">
              <button type="button" class="btn primary" disabled={!!props.busy || (!!p().existing && props.clash === 'cancel')} onClick={props.onInstall}>{props.busy ?? (p().update ? 'Install update' : 'Install skill')}</button>
              <button type="button" class="btn ghost" disabled={!!props.busy} onClick={props.onCancel}>Cancel</button>
            </div>
          </div>
        );
      }}</Show>
      <Show when={props.error}><ErrorText class="skill-import-error" text={props.error ?? ''} /></Show>
      <Show when={suggestedRename(props.error)}>{name => (
        <div class="skill-form-actions">
          <button type="button" class="btn primary" disabled={!!props.busy} onClick={() => props.onRename(name())}>Import as {name()}</button>
        </div>
      )}</Show>
    </div>
  );
}

function SkillDetail(props: {
  entry: CatalogEntry;
  selected: Skill;
  content: string;
  /** Why the skill's file couldn't be read, raw; null when it was. */
  readError: string | null;
  agent: string;
  agentLabel: string;
  onSelect: (s: Skill) => void;
  /** "Use in <workspace>", or "Use in a new workspace…" with none open. */
  useLabel: string;
  onUse: (s: Skill) => void;
  onEdit: (s: Skill) => void;
  onDelete: (s: Skill) => void;
  onCheckUpdate: (name: string) => void;
}) {
  const current = () => props.entry.copies.find(c => c.path === props.selected.path) ?? props.entry.copies[0];
  return (
    <div class="skill-detail">
      <header class="skill-detail-head">
        <div>
          <h2>/{props.entry.name}</h2>
          <Show when={props.entry.description} fallback={<p class="work-muted">No description in this skill’s file.</p>}><p class="skill-detail-desc">{props.entry.description}</p></Show>
        </div>
        <div class="skill-detail-actions">
          <button class="btn primary" onClick={() => props.onUse(current())}>{props.useLabel}</button>
        </div>
      </header>
      <p class="skill-availability" role="note" data-auto={skillAvailability(props.entry, props.agent, props.agentLabel).auto ? 'true' : 'false'}>
        <strong>In a {props.agentLabel} conversation:</strong> {skillAvailability(props.entry, props.agent, props.agentLabel).text}
      </p>
      <Show when={precedenceNote(props.entry)}>{note => <p class="skill-precedence" role="note">{note()}</p>}</Show>
      <h3 class="skill-subhead">{props.entry.copies.length > 1 ? 'Copies' : 'Where it comes from'}</h3>
      <ol class="skill-copies">
        <For each={props.entry.copies}>{c =>
          <li classList={{ selected: c.path === current().path }}>
            <button type="button" class="skill-copy" aria-pressed={c.path === current().path} onClick={() => props.onSelect(c)}>
              <span class={`skills-badge ${skillOwner(c).kind}`}>{skillOwner(c).badge}</span>
              <span class="skill-copy-text"><span>{skillOwner(c).usedBy}</span><small title={c.path}>{c.path.replace(/^\/Users\/[^/]+/, '~')}{c.version ? ` · v${c.version}` : ''} · {skillRuntime(c) === props.agent ? `${props.agentLabel} may load this copy by itself` : `${props.agentLabel}: on request`}</small></span>
            </button>
            <Show when={c.writable}><span class="skill-copy-actions"><button class="btn ghost small" onClick={() => props.onEdit(c)}>Edit skill</button><button class="btn ghost small" onClick={() => props.onDelete(c)}>Delete skill</button></span></Show>
          </li>
        }</For>
      </ol>
      {/* Only a copy Hollow manages (its folder is its name, not a link):
          the record, update and note go by name (review of #1050). */}
      <Show when={current().source === 'shared' && current().writable}><SkillSource skill={current()} onCheckUpdate={props.onCheckUpdate} /></Show>
      <Show when={props.readError}>{e => <ErrorText class="skill-import-error skill-read-error" text={`Couldn’t read this skill: ${e()}`} />}</Show>
      <Show when={current().lint}>{l => <>
        <h3 class="skill-subhead">Skill check</h3>
        <p class="skill-lint-head" role="note" data-grade={l().grade}><strong>{lintBadge(l()).label}</strong> · {l().grade}. A static check of the file: no agent was run and nothing was sent anywhere.</p>
        <Show when={l().findings.length} fallback={<p class="work-muted">No findings.</p>}>
          <ul class="skill-lint-list">
            <For each={lintFindingsInOrder(l())}>{f => <li data-area={f.area}>
              <span class="skills-badge">{f.area}</span> {f.message}
              <Show when={f.suggestion}>{s => <small class="skill-lint-fix">Fix: {s()}</small>}</Show>
            </li>}</For>
          </ul>
        </Show>
      </>}</Show>
      <SkillPortability path={current().path} rev={props.content} />
      <details class="skill-instructions">
        <summary>Instructions{props.entry.copies.length > 1 ? ` · ${skillOwner(current()).badge} copy` : ''}</summary>
        <MarkdownView text={stripFrontmatter(props.content)} class="skill-md" breaks={false} />
      </details>
    </div>
  );
}

/** NV3: whether each agent loads this skill, loads it degraded or doesn't,
 *  from the backend's per-agent profiles (`skill_portability.rs`). Read-only:
 *  it runs no agent and sends nothing. A rule recorded against another CLI
 *  version than the installed one says "not verified on this version". */
function SkillPortability(props: { path: string; rev: string }) {
  const [table] = createResource(() => [props.path, props.rev] as const, ([path]) => api.skillPortabilityTable(path));
  return (
    <>
      <h3 class="skill-subhead">Agents</h3>
      <Show when={table.error ? undefined : table()} fallback={<p class="work-muted">{table.error ? `Couldn’t read the agent table: ${errMsg(table.error)}` : 'Reading the agent table…'}</p>}>{t => <>
        <p class="skill-portability-head" role="note">{tableSummary(t().agents)} A check of how the skill reaches each agent: no agent was run and nothing was sent anywhere.</p>
        <table class="skill-portability">
          <thead><tr><th scope="col">Agent</th><th scope="col">Loads</th><th scope="col">What changes</th><th scope="col">Checked on</th></tr></thead>
          <tbody>
            <For each={t().agents}>{row => <tr data-effect={row.summary ?? 'unknown'}>
              <th scope="row">{row.label}</th>
              <td><span class="skills-badge skill-portability-effect" data-effect={row.summary ?? 'unknown'}>{row.summary ? effectLabel(row.summary) : 'Not assessed'}</span></td>
              <td>
                <For each={findingsOf(row, 'hollow')} fallback={<span class="work-muted">Nothing cut.</span>}>{f => <span class="skill-portability-note"><code>{f.rule}</code> {f.sentence}</span>}</For>
                <For each={findingsOf(row, 'native')}>{f => <span class="skill-portability-note work-muted"><code>{f.rule}</code> {effectLabel(f.effect)}: {f.sentence}</span>}</For>
              </td>
              <td>
                <span class="skill-portability-version">{versionLine(row)}</span>
                <Show when={row.stale}><span class="skills-badge skill-portability-stale">Not verified on this version</span></Show>
              </td>
            </tr>}</For>
          </tbody>
        </table>
        <details class="skill-portability-rules">
          <summary>Rules checked, per agent</summary>
          <For each={t().agents}>{row => <section>
            <h4>{row.label}</h4>
            <ul>
              <For each={rulesInOrder(row)}>{r => <li title={r.unverifiedReason ?? undefined}>
                <code>{r.id}</code> {r.what} · {effectLabel(r.effect)} when it applies · {r.channel === 'hollow' ? 'Hollow’s delivery' : 'the CLI’s own loader'} · {statusTag(r.status, r.verifiedOn)}
                <Show when={r.unverifiedReason}>{why => <small class="skill-lint-fix">Why not verified: {why()}</small>}</Show>
              </li>}</For>
            </ul>
          </section>}</For>
        </details>
      </>}</Show>
    </>
  );
}

// Bumped after an install, so an open Source block reads its record again.
const [provenanceTick, bumpProvenance] = createSignal(0);

/** Where an import record says a skill came from, in words. */
function sourceWords(r: NonNullable<SkillProvenance['record']>): string {
  if (r.origin) {
    const repo = r.origin.url.replace(/^https:\/\//, '').replace(/\.git$/, '');
    return `${repo}${r.origin.subdir ? ` (${r.origin.subdir})` : ''}${r.origin.commit ? ` at ${r.origin.commit.slice(0, 7)}` : ''}`;
  }
  return r.from.kind === 'path' ? r.from.path.replace(/^\/Users\/[^/]+/, '~') : `a dropped file, ${r.from.name}`;
}

const changeCount = (c: SkillFileChanges) => c.added.length + c.removed.length + c.modified.length;

/** Repo ideas 10-27 #2: a shared skill's Source (Hollow's import record in
 *  its app data, or "Not imported by Hollow", never a warning), what changed
 *  outside Hollow since, Check for update… (Import's card, nothing replaced
 *  before its Install) and its local note. */
function SkillSource(props: { skill: Skill; onCheckUpdate: (name: string) => void }) {
  const [view, { mutate }] = createResource(() => [props.skill.name, provenanceTick()] as const, ([name]) => api.skillProvenance(name));
  // A failed read must not be read: Solid's resource throws its error.
  const record = () => (view.error ? undefined : view());
  const [draft, setDraft] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [noteError, setNoteError] = createSignal<string | null>(null);
  createEffect(() => { void props.skill.name; setDraft(null); setNoteError(null); });
  async function saveNote(text: string | null) {
    // The answer belongs to this skill and this draft: one that comes back
    // after another skill was chosen, or after more typing, changes nothing
    // on screen (review of #1050).
    const name = props.skill.name;
    const sent = draft();
    setSaving(true);
    setNoteError(null);
    try {
      const next = await api.setSkillNote(name, text);
      if (props.skill.name !== name) return;
      mutate(next);
      if (draft() === sent) setDraft(null);
      flashFooter(text ? 'Local note saved' : 'Local note removed', 1500);
    } catch (err) { if (props.skill.name === name) setNoteError(errMsg(err)); } finally { setSaving(false); }
  }
  return (
    <section class="skill-source" aria-label="Source">
      <h3 class="skill-subhead">Source</h3>
      <Show when={record()} fallback={<p class="work-muted">{view.error ? `Couldn’t read this skill’s import record: ${errMsg(view.error)}` : 'Reading the import record…'}</p>}>{v => <>
        <Show when={v().record} fallback={<p class="work-muted skill-source-none">Not imported by Hollow</p>}>{r => <>
          <p class="skill-source-from">Imported {new Date(r().importedAt).toLocaleDateString()} from <span title={r().origin?.url ?? (r().from.kind === 'path' ? (r().from as { path: string }).path : undefined)}>{sourceWords(r())}</span></p>
          <Show when={v().changed && changeCount(v().changed!) > 0 ? v().changed : null}>{c => (
            <details class="skill-source-changed">
              <summary><strong>Changed outside Hollow</strong>: {changeCount(c())} file{changeCount(c()) === 1 ? '' : 's'} since the import</summary>
              <ul><For each={[...c().modified.map(f => ['changed', f]), ...c().added.map(f => ['added', f]), ...c().removed.map(f => ['removed', f])].slice(0, 20)}>{([kind, f]) => <li><span class="skills-badge">{kind}</span> {f}</li>}</For></ul>
            </details>
          )}</Show>
          <Show when={v().error}>{e => <p class="work-muted">Couldn’t compare its files: {e()}</p>}</Show>
          <div class="skill-form-actions">
            <button type="button" class="btn small" disabled={!v().canCheck} onClick={() => props.onCheckUpdate(props.skill.name)}
              title={r().origin ? 'Fetches the source now and shows what would change; nothing is replaced before you install' : 'Reads the source again and shows what would change; nothing is replaced before you install'}>Check for update…</button>
          </div>
          <Show when={!v().canCheck}><p class="work-muted">{r().from.kind === 'dropped' ? 'It came from a dropped file: drop the new version on this page to update it.' : 'Its source is gone: import the new version with Import skill.'}</p></Show>
        </>}</Show>
        <details class="skill-note" open={!!v().note}>
          <summary>Local note{v().note ? '' : ' (none)'}</summary>
          <p class="work-muted">Written into this skill’s SKILL.md for every agent, and written again when an update replaces the skill.</p>
          <textarea rows={3} maxLength={4000} aria-label="Local note" value={draft() ?? v().note ?? ''} onInput={e => setDraft(e.currentTarget.value)} />
          <div class="skill-form-actions">
            <button type="button" class="btn small primary" disabled={saving() || draft() === null} onClick={() => void saveNote(draft()!.trim() || null)}>Save note</button>
            <Show when={v().note}><button type="button" class="btn small ghost" disabled={saving()} onClick={() => void saveNote(null)}>Remove note</button></Show>
          </div>
          <Show when={noteError()}>{e => <ErrorText class="skill-import-error" text={e()} />}</Show>
        </details>
      </>}</Show>
    </section>
  );
}
