// What the Preview pane says about the item a tab shows (UI clutter pass,
// 2026-11-06): its kind, a file's size and modified time, and its full path.
// They used to fill two header rows (a bare kind word over the tabs, and a
// line of kind, absolute path, size and time under them); now a tab's hover
// tooltip carries them, and a file's ••• menu lists them beside Copy path.
// Pure, so the node vitest suite can load it.
import type { ChatArtifactKind } from './chatArtifactKind';

/** A file's kind in words. */
export const KIND_NAME: Record<ChatArtifactKind, string> = { html: 'HTML', markdown: 'Markdown', svg: 'SVG', image: 'Image', video: 'Video', pdf: 'PDF', diagram: 'Diagram', code: 'Text', binary: 'File' };

/** The size and modified time of the file read last (null until it loads). */
export type PreviewFileFacts = { path: string; size: number; modifiedMs: number | null };

export function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function fmtModified(ms: number | null): string {
  if (!ms) return '';
  return new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

/** "Markdown · 980.5 KB · 3:06 PM", or only the kind before the file loads. */
export function fileDetailsLine(kind: ChatArtifactKind, facts?: PreviewFileFacts | null): string {
  const parts = [KIND_NAME[kind]];
  if (facts) {
    parts.push(fmtSize(facts.size));
    const at = fmtModified(facts.modifiedMs);
    if (at) parts.push(at);
  }
  return parts.join(' · ');
}

/** A file tab's tooltip: the details line, then the full path (an attached
 *  picture has no path, only where it came from). */
export function fileTabTitle(path: string, kind: ChatArtifactKind, facts?: PreviewFileFacts | null): string {
  if (path.startsWith('data:')) return 'Image from the conversation';
  return `${fileDetailsLine(kind, facts?.path === path ? facts : null)}\n${path}`;
}
