import { latestRequest, runLatest } from './latestRequest';
import { isRetiredMessageSource } from './workspaceContent';

export type PalettePreviewTarget = { title: string; kind: 'Workspace brief' | 'Note' | 'Task'; path?: string; body?: string; facts?: string };
export function previewablePath(path: string): boolean {
  return !path.startsWith('/') && !path.includes('\\') && !path.split('/').includes('..')
    && /\.md$/i.test(path) && !isRetiredMessageSource(path);
}
/** Plain, bounded excerpt. No markdown renderer, links or executable content. */
export function previewText(body: string): string {
  const text = body.replace(/^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/, '').trim();
  return text.length > 1800 ? `${text.slice(0, 1800)}…` : text;
}
export function createPalettePreview(read: (path: string) => Promise<string>) {
  const order = latestRequest();
  return {
    cancel: () => order.cancel(),
    load(path: string, receive: (result: { ok: true; value: string } | { ok: false; error: unknown }) => void) {
      order.cancel();
      if (!previewablePath(path)) { receive({ ok: false, error: 'No note preview' }); return; }
      runLatest(order, () => read(path), receive);
    },
  };
}
