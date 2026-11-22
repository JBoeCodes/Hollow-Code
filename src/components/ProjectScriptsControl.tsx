import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Icon } from '../icons';
import { Button, IconButton, PlainErrorText } from './ui';
import { holdFocus } from '../focusHold';
import { displayChord } from '../shortcuts';
import { MAX_SCRIPTS, SCRIPT_CHORD_KEYS, scriptChord, scriptChordProblem, type ProjectScript } from '../projectScripts';
import { loadProjectScripts, projectScripts, runProjectScript, saveProjectScripts, scriptEditor, setScriptEditor } from '../store/projectScripts';
import { errMsg } from '../utils';
import '../styles/project-scripts.css';

/** The card's Scripts section (4.5, A10): each script with Run and Edit, and Add script. A run
 *  opens its output in the Preview pane; a script's chord (⇧⌘ and a key) runs it from anywhere
 *  in the project. Every action here has a button; the chord is the shortcut. */
export function ProjectScriptsControl(props: { workspaceId: string }) {
  onMount(() => { void loadProjectScripts(props.workspaceId); });
  const scripts = () => projectScripts(props.workspaceId);
  return <section class="scripts-section" aria-label="Scripts">
    <div class="scripts-head"><h3>Scripts</h3>
      <Show when={scripts().length < MAX_SCRIPTS}><IconButton label="Add script" icon="plus" onClick={() => setScriptEditor({ workspaceId: props.workspaceId, scriptId: null })} /></Show></div>
    <For each={scripts()} fallback={<p class="scripts-empty">Name a command (test, dev, lint) to run it from here.</p>}>{s =>
      <div class="scripts-row">
        <IconButton label={`Run ${s.name}`} icon="play" onClick={() => void runProjectScript(props.workspaceId, s)} />
        <span class="scripts-row-name" title={s.command}>{s.name}</span>
        <Show when={s.chord}>{c => <kbd class="script-chord">{displayChord(c())}</kbd>}</Show>
        <IconButton label={`Edit ${s.name}`} icon="edit" onClick={() => setScriptEditor({ workspaceId: props.workspaceId, scriptId: s.id })} />
      </div>
    }</For>
    <Show when={scriptEditor()?.workspaceId === props.workspaceId ? scriptEditor() : null} keyed>{request => <ScriptEditor workspaceId={props.workspaceId} scriptId={request.scriptId} />}</Show>
  </section>;
}

function ScriptEditor(props: { workspaceId: string; scriptId: string | null }) {
  const all = () => projectScripts(props.workspaceId);
  const existing = () => all().find(s => s.id === props.scriptId) ?? null;
  const [name, setName] = createSignal(existing()?.name ?? '');
  const [command, setCommand] = createSignal(existing()?.command ?? '');
  const [chord, setChord] = createSignal(existing()?.chord ?? '');
  const [error, setError] = createSignal('');
  const [pending, setPending] = createSignal(false);
  let card!: HTMLFormElement;
  const close = () => { if (!pending()) setScriptEditor(null); };
  onMount(() => {
    const release = holdFocus(card);
    card.querySelector<HTMLInputElement>('input')?.focus();
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); close(); } };
    document.addEventListener('keydown', key, true);
    onCleanup(() => { release(); document.removeEventListener('keydown', key, true); });
  });
  const others = () => all().filter(s => s.id !== props.scriptId);
  /** Why a chord can't be picked (the option is disabled and says so), or null. */
  const problem = (c: string) => scriptChordProblem(c, others());
  async function save() {
    if (pending() || !name().trim() || !command().trim()) return;
    setPending(true); setError('');
    const next: ProjectScript = { id: existing()?.id ?? '', name: name(), command: command(), chord: chord() || null };
    try {
      await saveProjectScripts(props.workspaceId, existing() ? all().map(s => s.id === next.id ? next : s) : [...all(), next]);
      setScriptEditor(null);
    } catch (e) { setError(errMsg(e)); }
    finally { setPending(false); }
  }
  async function remove() {
    if (pending() || !existing()) return;
    setPending(true); setError('');
    try { await saveProjectScripts(props.workspaceId, all().filter(s => s.id !== props.scriptId)); setScriptEditor(null); }
    catch (e) { setError(errMsg(e)); }
    finally { setPending(false); }
  }
  return <Portal><div class="rbk-backdrop" onClick={e => { if (e.target === e.currentTarget) close(); }}>
    <form ref={card} class="rbk-card scripts-editor" role="dialog" aria-modal="true" aria-labelledby="script-editor-title" onSubmit={e => { e.preventDefault(); void save(); }}>
      <h2 id="script-editor-title">{existing() ? 'Edit script' : 'Add script'}</h2>
      <label>Name<input aria-label="Script name" value={name()} maxLength={40} placeholder="test" onInput={e => setName(e.currentTarget.value)} /></label>
      <label>Command<textarea aria-label="Script command" value={command()} maxLength={2000} placeholder="npm test" spellcheck={false} rows={3} onInput={e => setCommand(e.currentTarget.value)} /></label>
      <p class="scripts-chord-note">Runs in this project's folder when you click Run{chord() ? ` or press ${displayChord(chord())}` : ''}. Its output opens beside the chat.</p>
      <label>Shortcut <span>optional</span>
        <select aria-label="Script shortcut" value={chord()} onChange={e => setChord(e.currentTarget.value)}>
          <option value="">None</option>
          <For each={SCRIPT_CHORD_KEYS}>{k => <option value={scriptChord(k)} disabled={!!problem(scriptChord(k)) && chord() !== scriptChord(k)} title={problem(scriptChord(k)) ?? undefined}>{displayChord(scriptChord(k))}{problem(scriptChord(k)) ? ` (${problem(scriptChord(k))!.replace(/^.* is /, '').replace(/\.$/, '')})` : ''}</option>}</For>
        </select></label>
      <Show when={error()}><p class="scripts-error" role="alert"><PlainErrorText text={error()} /></p></Show>
      <div class="work-actions">
        <Button variant="primary" type="submit" disabled={pending() || !name().trim() || !command().trim()}>{pending() ? 'Saving…' : 'Save'}</Button>
        <Show when={existing()}><Button disabled={pending()} onClick={() => void remove()}><Icon name="trash" size={14} />Delete</Button></Show>
        <Button variant="ghost" disabled={pending()} onClick={close}>Cancel</Button>
      </div>
    </form>
  </div></Portal>;
}
