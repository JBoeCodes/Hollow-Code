import { createEffect, createRoot, createSignal, on } from 'solid-js';
import { api } from '../api';
import { errMsg } from '../utils';
import { flashFooter } from './snackbars';
import { openPreview } from './preview';
import { workspaceId, workspaces } from './workspaces';
import { recordingShortcut, shortcutsEnabled, type ChordEvent } from '../shortcuts';
import { scriptForChord, type ProjectScript } from '../projectScripts';

/** The open project's scripts (4.5, A10), by project id. The list lives in the project's own
 *  `scripts.json`; this is the window's copy, read when a project opens, when the card shows it
 *  and after every save. */
const [byProject, setByProject] = createSignal<Record<string, ProjectScript[]>>({});
export const projectScripts = (id: string | null | undefined): ProjectScript[] => (id ? byProject()[id] ?? [] : []);

export async function loadProjectScripts(id: string): Promise<ProjectScript[]> {
  try {
    const list = await api.listProjectScripts(id);
    setByProject(m => ({ ...m, [id]: list }));
    return list;
  } catch {
    // A built-in workspace has none; either way don't ask again until the project is opened anew.
    setByProject(m => ({ ...m, [id]: m[id] ?? [] }));
    return projectScripts(id);
  }
}

/** Save a project's scripts; the backend's reason comes back as the error. */
export async function saveProjectScripts(id: string, scripts: ProjectScript[]): Promise<ProjectScript[]> {
  const saved = await api.saveProjectScripts(id, scripts);
  setByProject(m => ({ ...m, [id]: saved }));
  return saved;
}

/** Run one script and show its output in the Preview pane. */
export async function runProjectScript(id: string, script: ProjectScript): Promise<boolean> {
  try {
    await api.runProjectScript(id, script.id, script.command);
  } catch (e) {
    flashFooter(`× ${script.name} didn’t start: ${errMsg(e)}`, 6000);
    // The file changed under us: show what it holds now.
    if (/changed on disk/i.test(errMsg(e))) void loadProjectScripts(id);
    // A run that is already going still opens its output.
    if (!/already running/i.test(errMsg(e))) return false;
  }
  openPreview({ kind: 'script', workspaceId: id, scriptId: script.id });
  return true;
}

/** The script a ⌘⇧ keystroke runs in the open project, or null (and nothing while the keymap is
 *  off or a shortcut is being recorded). */
export function scriptForKeystroke(e: ChordEvent & { repeat?: boolean }): { workspaceId: string; script: ProjectScript } | null {
  const id = workspaceId();
  // A held key repeats; only its first press runs a script.
  if (!id || e.repeat || !shortcutsEnabled() || recordingShortcut()) return null;
  const script = scriptForChord(e, projectScripts(id));
  return script ? { workspaceId: id, script } : null;
}

// Read a project's scripts whenever it opens, so its chords work before the card is looked at.
createRoot(() => {
  createEffect(on([workspaceId, workspaces], ([id]) => { if (id && byProject()[id] === undefined) void loadProjectScripts(id); }));
});

/** The script editor's request (the card's Add script, a row's Edit, the header's Add project script):
 *  `scriptId` null adds one. The card's `ProjectScriptsControl` shows the dialog. */
export const [scriptEditor, setScriptEditor] = createSignal<{ workspaceId: string; scriptId: string | null } | null>(null);
