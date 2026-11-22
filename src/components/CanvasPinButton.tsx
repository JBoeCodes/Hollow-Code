import { createSignal, Show, onCleanup, onMount } from 'solid-js';
import type { Canvas } from '../generated/ipc';
import { sha256Hex } from '../sectionMarkdown';
import { api } from '../api';
import { canvasPins, removeCanvasPin, saveCanvasPin } from '../store/canvasPins';
import { openWorkView } from '../store/destinations';
import { liveDeclarations } from '../canvas/liveBindings';
import { ActionMenu } from './ActionMenu';
import { Icon } from '../icons';
import { ErrorText } from './ui';

/** Only a deliberate user action creates or replaces a pin. */
export function CanvasPinButton(props: { canvas: Canvas; align?: 'start' | 'end' }) {
  const [busy, setBusy] = createSignal(false), [error, setError] = createSignal('');
  const [vault, setVault] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<'home' | 'workspace' | null>(null);
  let dead = false, timer: ReturnType<typeof setTimeout> | undefined;
  onMount(() => { void api.setupStatus().then(s => { if (!dead) setVault(s.vaultPath); }).catch(() => {}); });
  onCleanup(() => { dead = true; clearTimeout(timer); });
  const existing = (destination: 'home' | 'workspace') => canvasPins().find(p => p.vault === vault() &&
    p.workspaceId === props.canvas.workspaceId && p.sessionKey === (props.canvas.workspaceId ? null : props.canvas.sessionKey) &&
    p.id === props.canvas.id && p.destination === destination);
  const pin = async (destination: 'home' | 'workspace') => {
    if (busy()) return;
    const c = props.canvas;
    setBusy(true); setError(''); setNotice(null); clearTimeout(timer);
    try {
      const root = (await api.setupStatus()).vaultPath;
      if (dead || props.canvas !== c) return;
      setVault(root);
      if (!root || c.htmlChanged || sha256Hex(c.html) !== c.fenceSha256) throw Error('Open a saved, unchanged revision to pin it.');
      saveCanvasPin({ vault: root, workspaceId: c.workspaceId, sessionKey: c.workspaceId ? null : c.sessionKey,
        id: c.id, revision: c.revision, sha256: c.fenceSha256, title: c.title, destination });
      setNotice(destination);
      timer = setTimeout(() => setNotice(null), 4000);
    } catch (e) { if (!dead) setError(e instanceof Error ? e.message : 'Could not save this pin.'); }
    finally { if (!dead) setBusy(false); }
  };
  const unpin = (destination: 'home' | 'workspace') => {
    const p = existing(destination); if (!p) return;
    try { removeCanvasPin(p); setError(''); setNotice(null); clearTimeout(timer); }
    catch { setError('Could not save this change. Try unpinning again.'); }
  };
  const show = async () => {
    try { await openWorkView('home'); }
    catch { if (!dead) setError('The pin’s destination could not be opened.'); }
  };
  const live = () => liveDeclarations(props.canvas.html).keys.length > 0 && !props.canvas.ask;
  const items = (destination: 'home') => {
    const place = 'Projects';
    return [
      { label: existing(destination) ? `Replace pin in ${place}` : `Pin${live() ? ' with live data' : ''} to ${place}`,
        disabled: busy() || props.canvas.htmlChanged, action: () => void pin(destination) },
      ...(existing(destination) ? [{ label: `Unpin from ${place}`, disabled: busy(), action: () => unpin(destination) }] : []),
    ];
  };
  return <div class="canvas-pin-action">
    <ActionMenu label={`Pin ${props.canvas.title}`} align={props.align} icon={<Icon name="pin" size={13}/>}
      items={[...items('home'),
        // Workspace pins showed in the Overview section, gone with the
        // workspace panel (COO rebuild L4): one made before can be unpinned.
        ...(existing('workspace') ? [{ label: 'Unpin from this workspace', disabled: busy(), action: () => unpin('workspace') }] : [])]}/>
    <Show when={notice()}><span role="status"><span>Pinned to Projects.</span>{' '}
      <button type="button" class="chat-canvas-back" onClick={() => void show()}>Show</button>
    </span></Show>
    <Show when={error()}><ErrorText class="chat-canvas-note" text={error()} /></Show>
  </div>;
}
