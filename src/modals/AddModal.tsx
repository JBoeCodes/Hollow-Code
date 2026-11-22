import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { createExitPhase } from '../components/exit';
import { api } from '../api';
import { Icon } from '../icons';
import { Button, ErrorText } from '../components/ui';
import { Select } from '../components/Select';
import { CAPTURE_LABEL, classifyFile, classifyText, ingestSourceFor, isWebAddress, noteName, type CaptureKind } from '../captureClassify';
import { activePage, flashFooter, refetchTasks, refetchVaultTree, refreshNoteIndex, openVaultNote, trackTask } from '../store';
import { flashWithUndo } from '../store/snackbars';
import { modalOpen } from '../modalGuard';
import { projectWorkspaces, workspaceId, selectWorkspace } from '../store/workspaces';
import type { ParsedQuickAdd } from '../types';
import { errMsg } from '../utils';
import { latestRequest } from '../latestRequest';
import '../styles/add-sheet.css';

import { addOpen as isOpen, setAddOpen as setOpen, addSeed as seed, addPreset, addRequest, addSummoned, openImportSource } from './addState';
export { openAdd, openImportSource } from './addState';

/**
 * One Add entry (next-level P12). Type or paste anything, or choose a file:
 * the sheet says what it looks like (task, note, Library source, workspace
 * file, recording link) and exactly where it will go, and the user can
 * change either before saving. A failed save keeps the input. Import a
 * source is this sheet set to a Library source (S5): its Domain and Purpose
 * go to `ingest_run`, which runs in the Activity dock while the sheet closes
 * at once; a failed import puts the text back as the draft. Quick capture
 * (⌘⌥⇧T, the tray) is this sheet set to a task, its preview showing every
 * parsed field the old window's chips did. New note keeps its own form.
 */
export function AddModal() {
  const [text, setText] = createSignal('');
  const [file, setFile] = createSignal<string | null>(null);
  const [override, setOverride] = createSignal<CaptureKind | null>(null);
  const [target, setTarget] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  // Import a source's two optional anchors, and the kind this opening starts on.
  const [domain, setDomain] = createSignal('');
  const [purpose, setPurpose] = createSignal('');
  const [preset, setPreset] = createSignal<CaptureKind | null>(null);
  // Whether the person picked a kind in this sheet (a later opening then keeps
  // it), and whether a system-wide key or tray item opened it (S5 review).
  const [chose, setChose] = createSignal(false);
  const [summoned, setSummoned] = createSignal(false);
  // A failed import's kind and anchors, for its draft when the sheet next opens.
  let restored: { text: string; domain: string; purpose: string } | null = null;
  const [draftLoaded,setDraftLoaded]=createSignal(false);
  const [offeredSeed,setOfferedSeed]=createSignal('');
  const openings=latestRequest();
  const draftKey='hollow.add-sheet.draft';
  function readDraft(){try{return sessionStorage.getItem(draftKey)??'';}catch{return text();}}
  const [parsed, setParsed] = createSignal<{ for: string; value: ParsedQuickAdd } | null>(null);
  let input: HTMLTextAreaElement | undefined;
  let restore: HTMLElement | null = null;
  createEffect(on(isOpen, open => {
    openings.cancel();
    if(!open)return;
    restore=document.activeElement as HTMLElement|null;
    const saved=readDraft();
    setText(saved||seed());setOfferedSeed(saved&&seed()!==saved?seed():'');setDraftLoaded(true);
    setFile(null);setOverride(null);setError('');setPreset(addPreset());setDomain('');setPurpose('');
    setChose(false);setSummoned(addSummoned());
    if(restored&&text()===restored.text){setPreset('source');setDomain(restored.domain);setPurpose(restored.purpose);}
    restored=null;
    setTarget(workspaceId()&&projectWorkspaces().some(w=>w.id===workspaceId())?workspaceId()!:'');
    queueMicrotask(()=>input?.focus());
  }));
  // Opened again while open (⌘⌥⇧T or ⌘⌥⇧I from another app, S5 review): follow
  // the new kind unless one was picked here, keep the draft, take the keys back.
  createEffect(on(addRequest, () => {
    if(!isOpen())return;
    if(!chose()){setPreset(addPreset());setOverride(null);}
    setSummoned(addSummoned());setError('');
    const s=seed();if(s&&s!==text()){if(!text())setText(s);else setOfferedSeed(s);}
    queueMicrotask(()=>input?.focus());
  }, { defer: true }));
  createEffect(()=>{
    if(!draftLoaded())return;
    try{if(text())sessionStorage.setItem(draftKey,text());else sessionStorage.removeItem(draftKey);}
    catch{/* Keep the in-memory draft if session storage is unavailable. */}
  });
  function discardDraft(){setText('');setOfferedSeed('');setFile(null);setOverride(null);setError('');input?.focus();}
  const guess = createMemo(() => file() ? classifyFile(file()!.split('/').pop() ?? file()!) : classifyText(text()));
  const fileKinds: CaptureKind[] = ['file', 'source'];
  const textKinds: CaptureKind[] = ['task', 'note', 'source', 'recording'];
  const allowed = () => file() ? fileKinds : textKinds;
  // A choice wins, then the kind this opening was set to, then the guess.
  const fromPreset = () => !override() && !!preset() && allowed().includes(preset()!);
  const kind = (): CaptureKind => { const o = override(); if (o && allowed().includes(o)) return o; return fromPreset() ? preset()! : guess().kind; };
  // A click picks the kind for this sheet, so typing on doesn't undo it (and
  // Enter keeps saving a picked Task, S5 review).
  const choose = (k: CaptureKind) => { setOverride(k); setChose(true); setPreset(k); };
  const why = () => {
    const label = CAPTURE_LABEL[kind()].toLowerCase();
    if (override()) return `You chose ${label}.`;
    // Quick capture keeps its syntax line (the removed window's resting hint).
    if (fromPreset() && preset() === 'task') return 'Quick capture: fri 4pm · for 90m · p1 · #project · @label · every monday.';
    if (fromPreset() && preset() !== 'source') return `You chose ${label}.`;
    return fromPreset() ? 'Import a source: a link, text or file becomes source notes in Library.' : `Looks like a ${label}: ${guess().reason}.`;
  };
  // Parse a task line the way quick capture does, for the preview.
  let timer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => { const t = text().trim(); const k = kind(); clearTimeout(timer); if (k !== 'task' || !t) return; timer = setTimeout(() => { api.parseQuickAdd(t).then(v => { if (text().trim() === t) setParsed({ for: t, value: v }); }).catch(() => {}); }, 160); });
  onCleanup(() => clearTimeout(timer));
  const close = () => { setOpen(false); queueMicrotask(() => restore?.focus?.()); };
  onMount(() => { const k = (e: KeyboardEvent) => { if (isOpen() && e.key === 'Escape' && !e.defaultPrevented) { e.preventDefault(); close(); } }; document.addEventListener('keydown', k); onCleanup(() => document.removeEventListener('keydown', k)); });
  const needsWorkspace = () => kind() === 'file' || kind() === 'recording';
  const canSave = () => !busy() && (!!text().trim() || !!file()) && !(needsWorkspace() && !target());
  // Enter saves where the quick windows did (S5 review): a task this sheet was
  // set to or that was picked, and one web address as a Library source.
  // Shift+Enter adds a line; a note, a guessed task and other text keep Enter
  // for new lines, so a note's first line is never saved as a task.
  const enterSaves = () => (kind() === 'task' && (fromPreset() || !!override())) || (kind() === 'source' && isWebAddress(text()));
  const saveVerb = () => kind() === 'source' ? 'imports' : 'adds';
  const destination = () => {
    const w = projectWorkspaces().find(x => x.id === target())?.title;
    switch (kind()) {
      case 'task': return 'Tasks · a local task file (tasks/)';
      case 'note': return `Library · library/${noteName(text())}.md`;
      case 'source': return file() ? `Library · source notes from ${file()!.split('/').pop()} (the original isn’t changed)` : isWebAddress(text()) ? 'Library · source notes from the public page (Hollow reads it signed out)' : 'Library · source notes from the pasted text';
      case 'file': return w ? `${w} · sources/ (a snapshot; the original isn’t changed)` : 'Choose a workspace for the snapshot';
      case 'recording': return w ? `${w} · a link to the recording (it stays in Plaud)` : 'Choose a workspace to link the recording to';
    }
  };
  async function pick() {
    const token=openings.peek();setError('');
    try{
      // Set to a Library source, the picker offers what an import reads first.
      const path=await openDialog(kind()==='source'
        ?{multiple:false,directory:false,title:'Choose a source file',filters:[{name:'Documents & images',extensions:['pdf','png','jpg','jpeg','webp','gif','md','txt','srt','vtt','epub','html']},{name:'All files',extensions:['*']}]}
        :{multiple:false,directory:false,title:'Choose a file to add'});
      // A kind picked here that suits a file (Library source) stays picked.
      if(openings.isLatest(token)&&isOpen()&&typeof path==='string'){setFile(path);if(override()&&!fileKinds.includes(override()!))setOverride(null);}
    }catch(e){if(openings.isLatest(token)&&isOpen())setError(errMsg(e));}
  }
  // Stepping back where a key or tray item summoned the sheet from (S5
  // review): armed as it closes, cancelled by any click, key or focus loss in
  // Hollow or a page change, and only then `go()`: 0.7 s after the save is
  // done (an import's, once it succeeded), so its notice is seen first.
  function armStepBack() {
    const page = activePage();
    let live = true; let wait: ReturnType<typeof setTimeout> | undefined;
    const cancel = () => {
      live = false; clearTimeout(wait);
      window.removeEventListener('pointerdown', cancel, true); window.removeEventListener('keydown', cancel, true); window.removeEventListener('blur', cancel);
    };
    window.addEventListener('pointerdown', cancel, true); window.addEventListener('keydown', cancel, true); window.addEventListener('blur', cancel);
    const go = () => {
      if (!live) return;
      wait = setTimeout(() => { const still = live && !isOpen() && activePage() === page; cancel(); if (still) void api.afterSummonedCapture(true).catch(() => {}); }, 700);
    };
    return { go, cancel };
  }
  const recordingId = () => /plaud\.ai\/(?:file\/)?([A-Za-z0-9_-]{6,})/i.exec(text())?.[1] ?? /plaud:([A-Za-z0-9_-]{6,})/i.exec(text())?.[1] ?? null;
  async function save() {
    if (busy()) return;
    const token=openings.peek();
    const submittedText=text();const submittedFile=file();const submittedKind=kind();const submittedTarget=target();
    const ownsView=()=>openings.isLatest(token)&&isOpen();
    // A sheet a key or tray item opened never moves Hollow after a save: its
    // notice offers Open, and Hollow steps back where it was (S5 review).
    const fromKey=summoned();
    let step: ReturnType<typeof armStepBack> | null = null;
    const t = text().trim();
    setBusy(true); setError('');
    try {
      switch (submittedKind) {
        case 'task': {
          // The preview's pending parse would only repeat this one.
          clearTimeout(timer);
          const p = parsed()?.for === t ? parsed()!.value : await api.parseQuickAdd(t).catch(() => null);
          await api.createTask({ title: p?.title || t, status: 'Not started', priority: p?.priority ?? undefined, due: p?.due ?? undefined, dueTime: p?.dueTime ?? undefined, durationMin: p?.durationMin ?? undefined, recurrence: p?.recurrence ?? undefined, labels: p?.labels ?? [], project: p?.project ?? undefined });
          void refetchTasks(); flashFooter('Task added to Tasks.'); break;
        }
        case 'note': {
          const name = noteName(t); const path = `library/${name}.md`;
          const body = t.startsWith('#') ? `${t}\n` : `# ${name}\n\n${t.split('\n').slice(1).join('\n').trim() || ''}\n`;
          await api.vaultCreateNote(path, body); await refreshNoteIndex().catch(() => {});
          if(ownsView()&&!fromKey)openVaultNote(path);
          else flashWithUndo({ message: 'Note saved to Library.', kind: 'status', ttlMs: 8000, actionLabel: 'Open', undo: () => openVaultNote(path) });
          break;
        }
        case 'source': {
          // An import can run for minutes: it goes to the Activity dock and the
          // sheet closes now. A failure puts the text back as the draft unless
          // something new was typed since.
          const source = ingestSourceFor(submittedText, submittedFile);
          if (!source) throw new Error('Paste a link or text, or choose a file.');
          const label = source.kind === 'file' ? (source.displayName || 'file') : source.kind === 'url' ? source.url : 'pasted text';
          const anchors = { domain: domain().trim(), purpose: purpose().trim() };
          // From a key: step back only once the import has succeeded.
          if (fromKey) step = armStepBack();
          const imported = step;
          void trackTask('Import source', () => api.ingestRun(source, anchors), { detail: label, onSuccessResult: r => `${r.createdCount} note${r.createdCount === 1 ? '' : 's'}` })
            .then(r => { refetchVaultTree(); void refreshNoteIndex(); flashFooter(r.createdCount ? `Imported ${r.createdCount} source note${r.createdCount === 1 ? '' : 's'} into Library.` : 'Nothing was saved from that source.', 4000); imported?.go(); })
            .catch(e => {
              // Put the text back with its kind and anchors, unless something new was typed.
              const kept = !submittedFile && !text().trim();
              if (kept) {
                setText(submittedText);
                if (isOpen()) { setPreset('source'); setOverride(null); setChose(false); setDomain(anchors.domain); setPurpose(anchors.purpose); }
                else restored = { text: submittedText, ...anchors };
              }
              const reason = errMsg(e).trim();
              flashFooter(`× Import failed: ${reason}${/[.!?]$/.test(reason) ? '' : '.'}${kept ? ' Your text is back in Add.' : ''}`, 8000);
              // From a key: bring Hollow forward, with the draft open again
              // when nothing else holds the keyboard, so neither goes unseen.
              if (imported) {
                imported.cancel(); void api.afterSummonedCapture(false).catch(() => {});
                if (kept && !isOpen() && !modalOpen()) openImportSource();
              }
            });
          flashFooter('Importing into Library…', 2600);
          break;
        }
        case 'file': {
          if (!submittedTarget) throw new Error('Choose a workspace for the file.');
          await api.attachWorkspaceSource(submittedTarget, submittedFile!);
          if (ownsView() && !fromKey) { flashFooter('Attached as a source snapshot.'); await selectWorkspace(submittedTarget); }
          else flashWithUndo({ message: 'Attached as a source snapshot.', kind: 'status', ttlMs: 8000, actionLabel: 'Open', undo: () => { void selectWorkspace(submittedTarget); } });
          break;
        }
        case 'recording': { const id = recordingId(); if (!id) throw new Error('That doesn’t look like a Plaud recording link.'); if (!submittedTarget) throw new Error('Choose a workspace to link it to.'); await api.setRecordingLink(id, { workspaceId: submittedTarget }); flashFooter('Recording linked. It stays in Plaud.'); break; }
      }
      const unchanged=text()===submittedText&&file()===submittedFile;
      if(!submittedFile&&text()===submittedText)setText('');
      if(ownsView()&&unchanged){
        close();
        // Opened from another app or the tray: go back there, as the quick
        // windows did (lib.rs knows where, and does nothing when Hollow was
        // already in front). An import goes once it has succeeded.
        if(fromKey){setSummoned(false);if(submittedKind!=='source')armStepBack().go();}
      } else step?.cancel();
    } catch (e) {
      if(ownsView())setError(errMsg(e)); else flashFooter(`× Couldn't add: ${errMsg(e)}`);
    } finally { setBusy(false); }
  }
  const phase = createExitPhase(isOpen, 150);
  return <Show when={phase() !== 'closed'}>
    <div class="modal add-sheet" classList={{ 'is-closing': phase() === 'closing' }} onClick={e => { if ((e.target as HTMLElement).classList.contains('modal-backdrop')) close(); }}>
      <div class="modal-backdrop" />
      <div class="modal-content add-sheet-content" role="dialog" aria-modal="true" aria-labelledby="add-sheet-title">
        <div class="modal-head"><h2 id="add-sheet-title">Add</h2><button class="close-btn" type="button" aria-label="Close" title="Close (Escape)" onClick={close}>×</button></div>
        <form onSubmit={e => { e.preventDefault(); void save(); }}>
          <Show when={!file()} fallback={<div class="add-sheet-file"><Icon name="fileText" size={15} /><span title={file()!}>{file()!.split('/').pop()}</span><button type="button" class="btn small ghost" onClick={() => { setFile(null); setOverride(null); }}>Remove</button></div>}>
            <textarea ref={input} aria-label="What to add" rows={4} placeholder="A task, a note, a link, a Plaud recording link…" value={text()} onInput={e => { setText(e.currentTarget.value); setOverride(null); }} onKeyDown={e => {
              if (e.key !== 'Enter' || e.isComposing) return;
              if (e.metaKey || e.ctrlKey || (!e.shiftKey && !e.altKey && enterSaves())) { e.preventDefault(); if (canSave()) void save(); }
            }} />
          </Show>
          <Show when={text()}><p class="add-sheet-draft-hint">Unsent text is kept for this session. <button type="button" class="btn-link" disabled={busy()} onClick={discardDraft}>Discard draft</button></p></Show>
          <Show when={offeredSeed()}><p class="add-sheet-draft-hint">Your draft is kept. <button type="button" class="btn-link" disabled={busy()} onClick={()=>{setText(offeredSeed());setOfferedSeed('');setOverride(null);}}>Replace with selected text</button></p></Show>
          <div class="add-sheet-row"><Button size="sm" variant="ghost" icon="upload" onClick={() => void pick()}>Choose a file…</Button></div>
          <div class="add-sheet-kind" role="group" aria-label="Add as">
            <For each={allowed()}>{k => <button type="button" aria-pressed={kind() === k} onClick={() => choose(k)}>{CAPTURE_LABEL[k]}</button>}</For>
          </div>
          <p class="add-sheet-why">{why()}</p>
          <Show when={kind() === 'task' && parsed()?.for === text().trim() && parsed()}>{p => <p class="add-sheet-preview">“{p().value.title}”{p().value.due ? ` · due ${p().value.due}${p().value.dueTime ? ` ${p().value.dueTime}` : ''}` : ''}{p().value.durationMin ? ` · ${p().value.durationMin}m` : ''}{p().value.recurrence ? ` · ↻ ${p().value.recurrence}` : ''}{p().value.priority ? ` · ${p().value.priority}` : ''}{p().value.project ? ` · #${p().value.project}` : ''}{p().value.labels.length ? ` · @${p().value.labels.join(' @')}` : ''}</p>}</Show>
          <Show when={kind() === 'source'}>
            <div class="add-sheet-anchors">
              <label><span>Domain (optional)</span><input type="text" placeholder="e.g. ML research" value={domain()} onInput={e => setDomain(e.currentTarget.value)} /></label>
              <label><span>Purpose (optional)</span><input type="text" placeholder="What you're learning or building" value={purpose()} onInput={e => setPurpose(e.currentTarget.value)} /></label>
            </div>
          </Show>
          <Show when={needsWorkspace()}><div class="add-sheet-row"><Select size="sm" label="Workspace" placeholder="Choose a workspace…" value={target()} onChange={setTarget} options={projectWorkspaces().map(w => ({ value: w.id, label: w.title }))} /></div></Show>
          <p class="add-sheet-dest"><Icon name="arrowRight" size={12} /><span>Goes to: <strong>{destination()}</strong></span></p>
          <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}> Your input is still here.</ErrorText>}</Show>
          <div class="work-actions"><Button variant="primary" type="submit" disabled={!canSave()}>{busy() ? 'Adding…' : kind() === 'source' ? 'Import into Library' : `Add ${CAPTURE_LABEL[kind()].toLowerCase()}`}</Button><Button variant="ghost" onClick={close}>Cancel</Button><span class="add-sheet-hint">{enterSaves() ? `↩ ${saveVerb()} · ⇧↩ new line` : `⌘↩ ${saveVerb()}`}</span></div>
        </form>
      </div>
    </div>
  </Show>;
}
