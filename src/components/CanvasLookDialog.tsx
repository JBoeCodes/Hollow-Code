import { For, Show, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { namedLook } from '../store/canvasLook';
import { canvasLookTarget, setCanvasLookTarget } from '../store/canvasLookTarget';
import { customThemes, refetchCustomThemes } from '../store/theme';
import { refreshWorkspaces } from '../store/workspaces';
import { trackTask } from '../store/activityDock';
import { holdFocus } from '../focusHold';
import { errMsg } from '../utils';
import { Select } from './Select';
import { ErrorText } from './ui';
import { HtmlArtifact } from './HtmlArtifact';
import '../styles/canvas-look.css';

const SAMPLE = '<hc-stats><hc-stat label="Completed" value="12"></hc-stat><hc-stat label="In progress" value="3"></hc-stat></hc-stats><hc-chart type="bar" label="This week" height="100">Mon: 4\nTue: 8\nWed: 12</hc-chart><hc-callout title="Canvas look">Only this workspace’s canvases use these colours.</hc-callout>';

export default function CanvasLookDialog() {
  const target = canvasLookTarget()!;
  const [selected, setSelected] = createSignal(target.canvasTheme ?? '');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const entry = () => namedLook(selected());
  const look = () => entry()?.look ?? null;
  const meta = () => customThemes.latest?.find(t => t.name === selected());
  const options = createMemo(() => [
    { value: '', label: 'App theme' },
    ...(customThemes.latest ?? []).map(t => ({ value: t.name, label: t.name })),
    ...(selected() && !meta() ? [{ value: selected(), label: `${selected()} (unavailable)`, disabled: true }] : []),
  ]);
  const close = () => { if (!busy()) setCanvasLookTarget(null); };
  let card!: HTMLDivElement;
  onMount(() => {
    void refetchCustomThemes();
    const release = holdFocus(card);
    card.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !document.querySelector('.hs-pop')) { e.preventDefault(); e.stopImmediatePropagation(); close(); }
    };
    document.addEventListener('keydown', key, true);
    onCleanup(() => { release(); document.removeEventListener('keydown', key, true); });
  });
  async function save(name: string | null) {
    if (busy()) return;
    setBusy(true); setError('');
    try {
      await trackTask('Save canvas look', () => api.setWorkspaceCanvasTheme(target.id, name));
      await refreshWorkspaces();
      setCanvasLookTarget(null);
    } catch (e) { setError(errMsg(e)); }
    finally { setBusy(false); }
  }
  return <div class="rbk-backdrop" onClick={e => { if (e.target === e.currentTarget) close(); }}>
    <div class="rbk-card canvas-look-card" ref={card} role="dialog" aria-modal="true" aria-labelledby="canvas-look-title" tabIndex={-1}>
      <div class="rbk-head"><h2 id="canvas-look-title">Canvas look · {target.title}</h2><button class="btn ghost" disabled={busy()} onClick={close}>Close</button></div>
      <Select label="Canvas look" value={selected()} options={options()} onChange={setSelected} disabled={busy()} />
      <Show when={meta()}>{m => <div class="canvas-look-swatches" role="img" aria-label={`Background ${m().background}, text ${m().foreground}, accent ${m().accent}`}>
        <For each={[m().background, m().foreground, m().accent]}>{color => <span style={{ background: color }} title={color} />}</For>
      </div>}</Show>
      <Show when={entry()?.missing}><p role="status">“{selected()}” isn't on this Mac; canvases use the app theme.</p></Show>
      <HtmlArtifact headless code={SAMPLE} look={look} defaultMode="offline" localOnly initialHeight={280} />
      <Show when={look()?.adjusted.length}><div class="canvas-look-adjustments"><h3>Contrast adjustments</h3><ul>
        <For each={look()?.adjusted}>{a => <li>{a.token.replace('--app-', '')}: {a.before.toFixed(1)}:1 → {a.to} ({a.ratio.toFixed(1)}:1)</li>}</For>
      </ul></div></Show>
      <For each={look()?.warnings}>{warning => <p role="status">{warning}</p>}</For>
      <p class="rbk-note">Canvas source and the app theme stay unchanged. Custom themes live on this Mac.</p>
      <Show when={error()}><ErrorText class="canvas-look-error" text={error()} /></Show>
      <div class="rbk-foot">
        <Show when={target.canvasTheme}><button class="btn ghost" disabled={busy()} onClick={() => void save(null)}>Remove</button></Show>
        <button class="btn primary" disabled={busy() || (!!selected() && !look())} onClick={() => void save(selected() || null)}>{busy() ? 'Saving…' : 'Save'}</button>
      </div>
    </div>
  </div>;
}
