import { createSignal, onMount, onCleanup, Show } from 'solid-js';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { api } from './api';
import { MarkdownView } from './components/MarkdownView';
import { ErrorText } from './components/ui';
import type { PreviewFile } from './types';

// The markdown preview window — a decorated document viewer the OS opens when
// you double-click a `.md` file (Hollow registers as a handler via
// bundle.fileAssociations → RunEvent::Opened → md_open::open_preview_window).
// Storeless by design: it imports only `api`,
// never `./App` / `./store`. Each window finds its own file by passing its
// label (`md-preview-N`) back to the backend, which holds the label→path map.
export function MdPreview() {
  // Theme: main.tsx sets dataset.theme for every window (including this
  // storeless one) before the window branch, so the persisted light/dark
  // base mode is already applied by the time we render. (Custom-theme
  // accent colors aren't derived here — base mode is enough for a
  // document view.)
  const [file, setFile] = createSignal<PreviewFile | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<{ text: string; cls: string } | null>(null);
  const [saving, setSaving] = createSignal(false);
  const win = getCurrentWindow();

  async function closeWindow() {
    try { await win.close(); } catch { /* ignore */ }
  }

  async function load() {
    try {
      const f = await api.takePreviewFile(win.label);
      setFile(f);
      try { await win.setTitle(f.name); } catch { /* ignore */ }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    }
  }

  async function save() {
    const f = file();
    if (!f || f.inVault || saving()) return;
    setSaving(true);
    setStatus({ text: 'saving…', cls: '' });
    try {
      const res = await api.savePreviewToVault(win.label);
      setStatus({ text: `✓ saved to ${res.vaultRelPath}`, cls: 'success' });
      // Reflect reality: the file now lives in the vault, so the Save button
      // swaps to the badge and the cover path flips to the vault-relative one.
      setFile(f => (f ? { ...f, inVault: true, vaultRelPath: res.vaultRelPath } : f));
    } catch (e: any) {
      setStatus({ text: `✗ ${e?.message ?? 'save failed'}`, cls: 'error' });
    } finally {
      setSaving(false);
    }
  }

  onMount(() => {
    load();
    // Escape closes the window; Cmd/Ctrl+S saves a copy to the vault.
    const onKeydown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { closeWindow(); return; }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        save();
      }
    };
    document.addEventListener('keydown', onKeydown);
    onCleanup(() => document.removeEventListener('keydown', onKeydown));
  });

  return (
    <div class="md-preview">
      <header class="md-preview-bar">
        <div class="md-preview-meta">
          <span class="md-preview-name">{file()?.name ?? 'Markdown'}</span>
        </div>
        <div class="md-preview-actions">
          <Show when={status()}>
            <span class={`md-preview-status ${status()!.cls}`}>{status()!.text}</span>
          </Show>
          <Show when={file()}>
            <Show
              when={!file()!.inVault}
              fallback={<span class="md-preview-badge">✓ Already in your vault</span>}
            >
              <button class="btn primary" disabled={saving()} onClick={save}>
                {saving() ? 'Saving…' : 'Save to vault'}
              </button>
            </Show>
          </Show>
        </div>
      </header>
      <main class="md-preview-body">
        <Show when={error()}>
          <ErrorText class="md-preview-error" as="div" text={`Couldn’t open this file: ${error()}`} />
        </Show>
        <Show when={!error() && !file()}>
          <div class="md-preview-loading">
            <span class="md-preview-loading-arch" aria-hidden="true" />
            Opening…
          </div>
        </Show>
        <Show when={file()}>
          <article class="md-preview-sheet">
            <header class="doc-cover doc-cover--inline md-cover">
              <h1 class="md-cover-title">{file()!.name.replace(/\.(md|markdown|mdown|mkd)$/i, '')}</h1>
              <div class="md-cover-rule" aria-hidden="true" />
              <div class="md-cover-meta">
                <span class="md-cover-path">
                  {file()!.inVault && file()!.vaultRelPath ? `vault · ${file()!.vaultRelPath}` : file()!.path}
                </span>
                <span class="md-cover-stat">{wordCount(file()!.content)} words</span>
                <Show when={file()!.truncated}>
                  <span class="md-cover-stat trunc">truncated for display</span>
                </Show>
              </div>
            </header>
            <MarkdownView text={file()!.content} class="md-preview-md" breaks={false} />
          </article>
        </Show>
      </main>
    </div>
  );
}

const wordCount = (s: string) => (s.match(/\S+/g) ?? []).length;
