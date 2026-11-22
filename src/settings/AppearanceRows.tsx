// Settings, General, Appearance (L4, X18: extracted from
// pages/Settings.tsx): themes, glass, custom theme cards and editor.
import { Show, createSignal, createMemo, onMount, onCleanup, For } from 'solid-js';
import { theme, glass, customTheme, customThemes, refetchCustomThemes, setActiveTheme, matchSystem, themeSelection, beginThemeSelection } from '../store/theme';
import { draftContrast, previewPalette } from './themeContrast';
import { flashFooter, showConfirm } from '../store';
import { api } from '../api';
import { Spinner } from '../components/Spinner';
import type { CustomThemeMeta } from '../types';
import { SettingsSheet, openerRef } from './SettingsSheet';
import { Select } from '../components/Select';
import { themeMenuOptions, themeMenuValue, parseThemeChoice } from './themeMenu';
import { vscodeThemeToSource } from '../vscodeThemeImport';
import { CUSTOM_PREFIX, FONT_OPTIONS, cleanFontName, type FontKind } from '../fontChoice';
import { codeFont, setFont, uiFont } from '../store/fonts';
import CustomIconRows, { iconRowsNeedAttention } from './CustomIconRows';
import { registerDirty } from './settingsDirty';

// The themes folder is fixed for the process lifetime, so the lookup is
// memoized at module scope rather than per mount.
export let themesDirOnce: Promise<string> | null = null;

export function themesDirPathOnce(): Promise<string> {
  if (!themesDirOnce) themesDirOnce = api.themesDirPath();
  return themesDirOnce;
}

/// Picker for the active theme — a swatch-preview gallery. The stock
/// light + dark palettes always appear first, followed by any custom
/// `*.theme` files in `~/Library/Application Support/Hollow/themes/`.
/// Editing or creating a theme opens an inline textarea editor; save
/// round-trips through the backend parser so syntax errors surface
/// immediately.
export function AppearanceRows() {
  type EditorState = { mode: 'new' | 'edit'; name: string; content: string; originalName: string | null };
  const [editor, setEditor] = createSignal<EditorState | null>(null);
  // What the editor opened with: only a change to it is a draft a section
  // switch would drop (P1-21), and not while its save runs.
  let opened: EditorState | null = null;
  const [busy, setBusy] = createSignal(false);
  let disposed = false;
  onCleanup(() => { disposed = true; });
  registerDirty('custom-theme', () => {
    const e = editor();
    return !!e && !busy() && (e.name !== opened?.name || e.content !== opened?.content);
  }, 'Custom theme');
  const [editorError, setEditorError] = createSignal<string | null>(null);
  const [importNote, setImportNote] = createSignal<string | null>(null);
  let importInput: HTMLInputElement | undefined;
  const [themesPath, setThemesPath] = createSignal<string | null>(null);
  // Once per session, not once per visit. SettingsSection unmounts its body on
  // every section switch, so a call in the component body re-fired each time
  // the user came back to Interface — and the path is fixed for the process.
  onMount(() => {
    themesDirPathOnce().then(setThemesPath).catch(() => { /* the path note is optional */ });
  });

  const activeCustomName = () => customTheme()?.name ?? null;
  const current = () => themeMenuValue({ matchSystem: matchSystem(), customName: activeCustomName(), glass: glass(), theme: theme() });
  const options = createMemo(() => themeMenuOptions(customThemes() ?? []));
  // One menu for every look; a custom theme is read from disk first.
  function choose(value: string) {
    const choice = parseThemeChoice(value);
    if (!choice || value === current()) return;
    if (choice.kind === 'system') void setActiveTheme('system');
    else if (choice.kind === 'stock') void setActiveTheme(choice.mode);
    else if (choice.kind === 'glass') void setActiveTheme({ base: choice.base, glass: true });
    else { const meta = (customThemes() ?? []).find(t => t.name === choice.name); if (meta) void applyCustom(meta); }
  }
  // Edit themes opens a sheet (Settings PR 5): the custom themes, New theme,
  // the editor and the themes folder. A draft asks before the sheet drops it.
  const [sheet, setSheet] = createSignal(false);
  const opener = openerRef();
  async function closeSheet() {
    const e = editor();
    const draft = !!e && !busy() && (e.name !== opened?.name || e.content !== opened?.content);
    if (draft && !await showConfirm({ title: 'Discard theme changes?', body: 'Your saved themes stay unchanged.', confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true })) return;
    setEditor(null); setEditorError(null); setSheet(false);
  }

  async function applyCustom(meta: CustomThemeMeta, ev?: MouseEvent) {
    if (busy()) return;
    setBusy(true);
    let owner = beginThemeSelection();
    try {
      const t = await api.readCustomTheme(meta.name);
      if (!disposed && owner === themeSelection()) {
        const applying = setActiveTheme({ custom: t }, ev ? { clientX: ev.clientX, clientY: ev.clientY } : undefined);
        owner = themeSelection();
        await applying;
      }
    } catch (e: any) {
      if (!disposed && owner === themeSelection()) flashFooter(`× theme failed: ${e?.message || e}`, 3500);
    } finally {
      setBusy(false);
    }
  }

  function openNew() {
    setEditorError(null);
    opened = {
      mode: 'new',
      name: 'New Theme',
      content: NEW_THEME_TEMPLATE,
      originalName: null,
    };
    setEditor(opened);
  }

  /** A VS Code theme file becomes a new theme in the editor, for review; nothing is saved until Save. */
  async function importVsCode(file: File | undefined) {
    if (!file) return;
    setEditorError(null);
    setImportNote(null);
    try {
      if (file.size > 2_000_000) throw new Error('That file is too large to be a colour theme.');
      const result = vscodeThemeToSource(await file.text(), file.name);
      opened = { mode: 'new', name: result.name, content: result.source, originalName: null };
      setEditor(opened);
      setImportNote(`Imported “${result.name}”. ${result.notes.join(' ')} Check the colours, then Save.`.trim());
    } catch (e: any) {
      setImportNote(null);
      flashFooter(`× ${e?.message || e}`, 4500);
    } finally {
      if (importInput) importInput.value = '';
    }
  }

  async function openEdit(meta: CustomThemeMeta) {
    setEditorError(null);
    setBusy(true);
    try {
      const t = await api.readCustomTheme(meta.name);
      opened = { mode: 'edit', name: t.name, content: t.source, originalName: t.name };
      setEditor(opened);
    } catch (e: any) {
      flashFooter(`× couldn't open theme: ${e?.message || e}`, 3500);
    } finally {
      setBusy(false);
    }
  }

  async function saveEditor() {
    const state = editor();
    if (!state || busy()) return;
    setEditorError(null);
    setBusy(true);
    try {
      const written = await api.writeCustomTheme(state.name.trim() || 'New Theme', state.content);
      // Rename support: if the editor renamed an existing theme, drop
      // the old file so we don't leave a stale duplicate.
      if (state.originalName && state.originalName !== written.name) {
        try { await api.deleteCustomTheme(state.originalName); } catch { /* best effort */ }
      }
      await refetchCustomThemes();
      // If this theme is currently active, re-apply so the changes show
      // up without requiring the user to pick it again.
      if (customTheme()?.name === state.originalName || customTheme()?.name === written.name) {
        await setActiveTheme({ custom: written });
      }
      setEditor(null);
      flashFooter(`Saved "${written.name}"`, 1800);
    } catch (e: any) {
      setEditorError(typeof e === 'string' ? e : e?.message || String(e));
    } finally {
      setBusy(false);
    }
  }

  async function deleteCustom(meta: CustomThemeMeta) {
    if (busy()) return;
    const ok = await showConfirm({
      title: `Delete the "${meta.name}" theme?`,
      body: 'The theme file is removed from disk immediately.',
      confirmLabel: 'Delete',
      cancelLabel: 'Keep',
      danger: true,
    });
    if (!ok) return;
    setBusy(true);
    try {
      await api.deleteCustomTheme(meta.name);
      // If the deleted theme was active, fall back to the matching
      // stock palette.
      if (customTheme()?.name === meta.name) {
        setActiveTheme(meta.isDark ? 'dark' : 'light');
      }
      await refetchCustomThemes();
      flashFooter(`Deleted "${meta.name}"`, 1800);
    } catch (e: any) {
      flashFooter(`× delete failed: ${e?.message || e}`, 3500);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <details class="settings-fold settings-icons">
        {/* The label keeps its own element so a lookup by its exact text (the route ledger's walk to Upload an icon)
            still finds it while the flag shows beside it. */}
        <summary><span>App and COO icons</span><Show when={iconRowsNeedAttention()}><span class="settings-icons-flag"> · Needs attention</span></Show></summary>
        <CustomIconRows />
      </details>
      <div class="settings-row settings-theme-row">
        <div class="settings-row-label">
          <div class="settings-row-k">Theme</div>
          <div class="settings-row-v">Match your Mac, choose a look or make your own.</div>
        </div>
        <div class="settings-actions">
          <Select label="Theme" value={current()} options={options()} searchable={false} align="end" onChange={choose} />
          <button class="btn" onClick={e => { opener.mark(e); setSheet(true); }}>Edit themes…</button>
        </div>
      </div>
      <Show when={sheet()}>
      <SettingsSheet id="settings-sheet-themes" title="Edit themes" returnTo={opener.get} locked={busy} onClose={() => void closeSheet()}>
      <div class="theme-list">
        <For each={customThemes() ?? []}>
          {(meta, i) => (
            <ThemeCard
              idx={Math.min(i() + 4, 12)}
              name={meta.name}
              mode={`custom — ${meta.isDark ? 'dark' : 'light'} · ${meta.path.split('/').slice(-2).join('/')}`}
              swatch={{ bg: meta.background, fg: meta.foreground, accent: meta.accent }}
              dark={meta.isDark}
              active={activeCustomName() === meta.name}
              onUse={(ev) => applyCustom(meta, ev)}
              onEdit={() => openEdit(meta)}
              onDelete={() => deleteCustom(meta)}
            />
          )}
        </For>
      </div>
      <Show when={!(customThemes() ?? []).length && !customThemes.loading}>
        <p class="settings-note">No themes of your own yet. New theme starts one from a template.</p>
      </Show>
      <Show when={customThemes.loading}>
        <p class="settings-loading"><Spinner  /> loading themes…</p>
      </Show>
      <div class="settings-actions" style="margin-top:8px;">
        <button class="btn" onClick={openNew} disabled={busy()}>New theme…</button>
        <button class="btn" onClick={() => importInput?.click()} disabled={busy()}>Import VS Code theme…</button>
        <input ref={importInput} type="file" accept=".json,.jsonc,application/json" hidden aria-label="VS Code theme file"
          onChange={e => void importVsCode(e.currentTarget.files?.[0])} />
        <button class="btn ghost" onClick={() => api.revealThemesFolder().catch(() => {})}>
          Open themes folder
        </button>
        <button class="btn ghost" onClick={() => refetchCustomThemes()} disabled={busy()}>
          Refresh
        </button>
      </div>
      <Show when={importNote()}><p class="settings-note" role="status">{importNote()}</p></Show>
      <Show when={themesPath()}>
        <p class="settings-note" style="font-family:var(--mono);font-size:var(--fs-caption);">
          Themes folder: {themesPath()}
        </p>
      </Show>
      <Show when={editor()}>
        <ThemeEditor
          state={editor()!}
          onChange={(s) => setEditor(s)}
          onCancel={() => { setEditor(null); setEditorError(null); }}
          onSave={saveEditor}
          busy={busy()}
          error={editorError()}
        />
      </Show>
      </SettingsSheet>
      </Show>
    </>
  );
}

/// One card in the theme gallery: a miniature page preview built from
/// the theme's own bg/fg/accent, with the name + mode + actions below.
/// The sample is passive; Use alone selects a theme and forwards its real
/// MouseEvent to seed the theme-change reveal origin.
export function ThemeCard(props: {
  idx: number;
  name: string;
  mode: string;
  swatch: { bg: string; fg: string; accent: string };
  active: boolean;
  glassy?: boolean;
  dark?: boolean;
  onUse: (ev: MouseEvent) => void;
  onEdit?: () => void;
  onDelete?: () => void;
}) {
  const sample = createMemo(() => previewPalette(props.swatch, !!props.dark));
  return (
    <div class={`theme-card ${props.active ? 'active' : ''} ${props.glassy ? 'glassy' : ''}`} style={{ '--i': Math.min(props.idx, 12) }}>
      <div
        class="theme-card-preview"
        style={{ background: sample()['--canvas'], color: sample()['--ink'] }}
        role="img" aria-label={`${props.name} theme sample: title, body text, selection and action`}
      >
        <span class="theme-preview-label">Sample</span>
        <span class="theme-preview-card" style={{ background: sample()['--surface-card'] }} aria-hidden="true">
          <strong>A clear next step</strong>
          <span>Room for your work.</span>
          <span class="theme-preview-selection" style={{ background: sample()['--surface-cream-strong'], color: sample()['--body-strong'], 'border-color': sample()['--primary'] }}>Selected item</span>
          <span class="theme-preview-action" style={{ background: sample()['--surface-raised'], 'border-color': sample()['--primary'] }}>Continue</span>
        </span>
      </div>
      <div class="theme-card-meta">
        <div class="theme-card-name">
          {props.name}
        </div>
        <div class="theme-card-mode">{props.mode}</div>
      </div>
      <div class="theme-card-actions">
        <Show when={props.onEdit}>
          <button class="btn ghost small" onClick={props.onEdit}>Edit</button>
        </Show>
        <Show when={props.onDelete}>
          <button class="btn ghost small" onClick={props.onDelete}>Delete</button>
        </Show>
        <button
          class="btn small"
          classList={{ ghost: props.active }}
          aria-label={`${props.active ? 'In use:' : 'Use'} ${props.name} theme`}
          aria-pressed={props.active}
          disabled={props.active}
          onClick={(e) => props.onUse(e)}
        >{props.active ? 'In use' : 'Use'}</button>
      </div>
    </div>
  );
}

export function ThemeEditor(props: {
  state: { mode: 'new' | 'edit'; name: string; content: string; originalName: string | null };
  onChange: (s: { mode: 'new' | 'edit'; name: string; content: string; originalName: string | null }) => void;
  onSave: () => void;
  onCancel: () => void;
  busy: boolean;
  error: string | null;
}) {
  const contrast = createMemo(() => draftContrast(props.state.content));
  return (
    <div class="settings-row settings-theme-editor" style="display:flex;flex-direction:column;align-items:stretch;gap:8px;border:1px solid var(--hairline);padding:12px;border-radius:var(--r-md);background:var(--surface-soft);">
      <p class="settings-section-hint">Kitty-style colours: <code>accent = &quot;#hex&quot;</code> or <code>accent #hex</code>.</p>
      <div style="display:flex;gap:8px;align-items:center;">
        <label class="settings-row-k" style="min-width:80px;">Filename</label>
        <input
          aria-label="Theme filename"
          class="settings-select"
          style="flex:1;"
          value={props.state.name}
          onInput={(e) => props.onChange({ ...props.state, name: (e.currentTarget as HTMLInputElement).value })}
          placeholder="My Theme"
        />
      </div>
      <textarea
        aria-label="Theme source"
        spellcheck={false}
        rows={18}
        style="font-family:var(--mono);font-size:var(--fs-meta);line-height:1.45;padding:10px;border:1px solid var(--hairline);background:var(--canvas);color:var(--ink);border-radius:var(--r-sm);resize:vertical;"
        value={props.state.content}
        onInput={(e) => props.onChange({ ...props.state, content: (e.currentTarget as HTMLTextAreaElement).value })}
      />
      <div class="settings-row-v" role="status" aria-label="Theme text contrast">
        <Show when={contrast()} fallback="Text contrast unavailable. Complete the required hex colors to measure this draft.">
          {result => <>Core text: {result().ratio.toFixed(2)}:1 minimum across app surfaces. AA met (4.5:1).
            {result().adjusted ? ' Display colors adjusted for readability; your source stays unchanged.' : ' No text adjustment needed.'}</>}
        </Show>
      </div>
      <Show when={props.error}>
        <div class="settings-row-warn" style="white-space:pre-wrap;">{props.error}</div>
      </Show>
      <div style="display:flex;gap:6px;justify-content:flex-end;">
        <button class="btn ghost" onClick={props.onCancel} disabled={props.busy}>Cancel</button>
        <button class="btn primary" onClick={props.onSave} disabled={props.busy}>
          {props.busy ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>
  );
}

export const NEW_THEME_TEMPLATE = `# Required: accent, foreground, background.
# Everything else is optional and falls back to derived values.
name = "New Theme"
is_dark = false

accent     = "#c8761a"
foreground = "#1c1408"
background = "#f4ecd8"
selection_foreground = "#1c1408"
selection_background = "#e8c98a"

color0  = "#f4ecd8"
color1  = "#b73d3d"
color2  = "#4a8f3f"
color3  = "#b8841a"
color4  = "#3a6e8a"
color5  = "#8a4a8a"
color6  = "#2f7a85"
color7  = "#14110a"
color8  = "#756849"
color9  = "#d34e4e"
color10 = "#6db05d"
color11 = "#e8a55a"
color12 = "#5a8aa6"
color13 = "#a16ba1"
color14 = "#5db8c4"
color15 = "#000000"
`;

const OTHER_FONT = '__other__';

/// Interface and code font (parity 4.7, A15): Hollow's own, a system stack, or
/// a family name typed in. A name that is not installed falls back to Hollow's.
function FontRow(props: { kind: FontKind; title: string; hint: string }) {
  const choice = () => props.kind === 'ui' ? uiFont() : codeFont();
  const typed = () => choice().startsWith(CUSTOM_PREFIX) ? choice().slice(CUSTOM_PREFIX.length) : '';
  const [other, setOther] = createSignal(false);
  const [draft, setDraft] = createSignal('');
  const options = () => [
    ...FONT_OPTIONS[props.kind].map(o => ({ value: o.value, label: o.label })),
    ...(typed() ? [{ value: choice(), label: typed() }] : []),
    { value: OTHER_FONT, label: 'Other font…' },
  ];
  function pick(value: string) {
    if (value === OTHER_FONT) { setDraft(typed()); setOther(true); return; }
    setOther(false);
    setFont(props.kind, value);
  }
  function commit() {
    const name = cleanFontName(draft());
    if (!name) { flashFooter('× Use letters, digits, spaces, dots and hyphens in a font name (up to 60 characters).', 3500); return; }
    setFont(props.kind, CUSTOM_PREFIX + name);
    setOther(false);
  }
  return (
    <div class="settings-row">
      <div class="settings-row-label">
        <div class="settings-row-k">{props.title}</div>
        <div class="settings-row-v">{props.hint}</div>
      </div>
      <div class="settings-actions">
        <Select label={props.title} value={other() ? OTHER_FONT : choice()} options={options()} searchable={false} align="end" onChange={pick} />
        <Show when={other()}>
          <input class="settings-select" aria-label={`${props.title} family name`} placeholder="Installed font name" value={draft()}
            onInput={e => setDraft(e.currentTarget.value)} onKeyDown={e => { if (e.key === 'Enter') commit(); }} />
          <button type="button" class="btn" onClick={commit}>Use font</button>
        </Show>
      </div>
    </div>
  );
}

export function FontRows() {
  return (
    <>
      <FontRow kind="ui" title="Text font" hint="The font for labels, lists and messages. Default is Hollow’s own." />
      <FontRow kind="code" title="Code font" hint="The font for code, files and the terminal-style text." />
    </>
  );
}
