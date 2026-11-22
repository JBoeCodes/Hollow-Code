import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { canvasExportBusy, canvasExportTarget, closeCanvasExport, setCanvasExportBusy } from '../store/canvasExport';
import type { ExportFormat, ExportPreset, ExportedFile } from '../canvas/canvasExport';
import { exportPalette } from '../canvas/exportLook';
import { customThemes, refetchCustomThemes } from '../store/theme';
import { workspaces, workspaceDetail, openWorkspaceResult } from '../store/workspaces';
import { namedLook } from '../store/canvasLook';
import { holdFocus } from '../focusHold';
import { errMsg } from '../utils';
import { Select } from './Select';
import { ErrorText } from './ui';
import '../styles/canvas-export.css';
import { canvasFlows, flowColors, flowSvg, resolvedFlowTheme } from '../canvas/flowSvg';
import { EXPORT_WIDTHS } from '../canvas/canvasExport';

export default function ExportCanvasDialog() {
  const [format, setFormat] = createSignal<ExportFormat>('png');
  const [preset, setPreset] = createSignal<ExportPreset>('doc');
  const [look, setLook] = createSignal('match');
  const workspaceTheme = () => workspaces().find(w => w.id === canvasExportTarget()?.ref?.workspaceId)?.canvasTheme;
  const themeName = () => look() === 'workspace' ? workspaceTheme() : look().startsWith('custom:') ? look().slice(7) : null;
  const entry = () => namedLook(themeName());
  const lookOptions = () => [
    { value: 'match', label: 'App theme' },
    { value: 'workspace', label: "This workspace's look" },
    { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark' },
    ...(customThemes.latest ?? []).map(t => ({ value: 'custom:' + t.name, label: t.name })),
    ...(look().startsWith('custom:') && !customThemes.latest?.some(t => t.name === themeName())
      ? [{ value: look(), label: `${themeName()} (unavailable)`, disabled: true }] : []),
  ];
  const [copy, setCopy] = createSignal(false);
  const [flowIndex, setFlowIndex] = createSignal('1');
  const flows = createMemo(() => canvasFlows(canvasExportTarget()?.source || ''));
  const [result, setResult] = createSignal<(ExportedFile & { format: ExportFormat }) | null>(null);
  const [error, setError] = createSignal('');
  let card: HTMLDivElement | undefined, release: (() => void) | undefined;
  createEffect(on(canvasExportTarget, target => {
    setResult(null); setError(''); setCopy(false); setFormat('png'); setFlowIndex('1');
    setLook('match');
    if (target) void refetchCustomThemes();
    release?.(); release = undefined;
    if (target) queueMicrotask(() => { if (card && canvasExportTarget() === target) { release = holdFocus(card); card.focus(); } });
  }));
  onCleanup(() => release?.());
  onMount(() => {
    const key = (e: KeyboardEvent) => { if (canvasExportTarget() && e.key === 'Escape') { e.preventDefault(); e.stopImmediatePropagation(); closeCanvasExport(); } };
    document.addEventListener('keydown', key, true);
    onCleanup(() => document.removeEventListener('keydown', key, true));
  });
  const save = async () => {
    const target = canvasExportTarget();
    if (!target?.ref?.workspaceId || canvasExportBusy()) return;
    setCanvasExportBusy(true); setError(''); setResult(null);
    try {
      const savedFormat = format();
      const savedLook = look() === 'light' || look() === 'dark' ? look() as 'light' | 'dark' : 'match';
      const palette = savedLook === 'match' ? await exportPalette(themeName()) : { scheme: savedLook, theme: [] };
      const { scheme, theme } = palette;
      const flow = flows()[Number(flowIndex()) - 1];
      if (savedFormat === 'svg' && !flow) throw new Error('Choose a flow to export.');
      const drawing = savedFormat === 'svg' ? flowSvg(flow, EXPORT_WIDTHS[preset()], flowColors(scheme, resolvedFlowTheme(theme))) : null;
      const outcome = await api.canvasExport({ ...target.ref, source: target.source, format: savedFormat, preset: preset(), look: savedLook, copy: savedFormat === 'png' && copy(), target: 'file', ...palette, ...(drawing ? { svg: drawing.svg, flowIndex: Number(flowIndex()) } : {}) });
      if (drawing?.warning) outcome.warning = [outcome.warning, drawing.warning].filter(Boolean).join(' ');
      if (canvasExportTarget() === target) setResult({ ...outcome, format: savedFormat });
    } catch (e) { if (canvasExportTarget() === target) setError(errMsg(e)); }
    finally { setCanvasExportBusy(false); }
  };
  return <Show when={canvasExportTarget()}><div class="rbk-backdrop" onClick={e => { if (e.target === e.currentTarget) closeCanvasExport(); }}>
    <div class="rbk-card canvas-export-card" ref={card} role="dialog" aria-modal="true" aria-labelledby="canvas-export-title" tabIndex={-1}>
      <div class="rbk-head"><h2 id="canvas-export-title">Export canvas</h2><button class="btn ghost" disabled={canvasExportBusy()} onClick={closeCanvasExport}>Close</button></div>
      <p>{canvasExportTarget()?.title}</p>
      <div class="canvas-export-field"><span>Format</span><Select label="Format" value={format()} disabled={canvasExportBusy()} onChange={v => setFormat(v as ExportFormat)} options={[{ value: 'png', label: 'PNG image' }, { value: 'pdf', label: 'PDF document' }, ...(flows().length ? [{ value: 'svg', label: 'SVG flow' }] : [])]} /></div>
      <Show when={format() === 'svg'}><div class="canvas-export-field"><span>Flow</span><Select label="Flow" value={flowIndex()} disabled={canvasExportBusy()} onChange={setFlowIndex} options={flows().map((f, i) => ({ value: String(i + 1), label: `${i + 1}. ${f.label}` }))} /></div></Show>
      <div class="canvas-export-field"><span>Size</span><Select label="Size" value={preset()} disabled={canvasExportBusy()} onChange={v => setPreset(v as ExportPreset)} options={[{ value: 'email', label: 'Email · 600 px' }, { value: 'doc', label: 'Doc · 800 px' }, { value: 'slide', label: 'Slide · 1280 px' }]} /></div>
      <div class="canvas-export-field"><span>Look</span><Select label="Look" value={look()} disabled={canvasExportBusy()} onChange={setLook} options={lookOptions()} /></div>
      <Show when={look() === 'workspace' && !workspaceTheme()}><p class="rbk-note">This workspace uses the app theme.</p></Show>
      <Show when={entry()?.missing}><p role="status">“{themeName()}” is unavailable on this Mac. Choose App theme or another look to export.</p></Show>
      <Show when={themeName() && !entry()?.look && !entry()?.missing}><p role="status">Loading canvas look…</p></Show>
      <Show when={entry()?.look?.adjusted.length}><p class="rbk-note">{entry()!.look!.adjusted.length} contrast adjustments from the workspace palette rules.</p></Show>
      <For each={entry()?.look?.warnings}>{warning => <p class="rbk-note">{warning}</p>}</For>
      <label><input type="checkbox" checked={copy()} disabled={canvasExportBusy() || format() !== 'png'} onChange={e => setCopy(e.currentTarget.checked)} /> Also copy to clipboard (PNG)</label>
      <p class="rbk-note">{format() === 'svg' ? 'One static SVG per selected flow, from its saved text. Other content and script changes are left out.' : 'Drawn with scripts on and no internet; remote content is left out.'} Saved to this workspace’s Results.</p>
      <Show when={error()}><ErrorText class="canvas-export-error" text={error()} /></Show>
      <Show when={result()}>{r => <div role="status"><p>Saved {r().rel}{r().copied ? ' · Copied to clipboard' : ''}{r().format === 'png' ? ` · ${r().scale}× pixels` : ''}</p>
        <Show when={r().warning}><p>{r().warning}</p></Show>
        <button class="btn ghost" onClick={() => { if (r().path) void api.chatPreviewOpen(r().path!, true).catch(e => setError(errMsg(e))); }}>Reveal in Finder</button>
        <button class="btn ghost" disabled={!r().rel} onClick={() => {
          const target = canvasExportTarget(), path = r().rel, workspace = target?.ref?.workspaceId;
          if (!workspace || !path) return;
          // The saved file opens in the Files tab, with its review bar (COO rebuild L4).
          const folder = workspaces().find(w => w.id === workspace)?.path;
          if (!folder) return;
          void openWorkspaceResult(workspace, `${folder}/${path}`).then(() => { if (canvasExportTarget() === target) closeCanvasExport(); })
            .catch(e => { if (canvasExportTarget() === target) setError(errMsg(e)); });
        }}>Show the file</button></div>}</Show>
      <div class="rbk-foot"><button class="btn primary" disabled={canvasExportBusy() || !canvasExportTarget()?.ref?.workspaceId || (!!themeName() && !entry()?.look)} onClick={() => void save()}>{canvasExportBusy() ? 'Saving…' : 'Save'}</button></div>
    </div>
  </div></Show>;
}
