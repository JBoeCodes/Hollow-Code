import { isVideoPath } from './chatFileLinks';

/** How the side pane shows a conversation file (re-exported by the store;
 *  its own module so tests can import it without the store). */
export type ChatArtifactKind = 'html' | 'markdown' | 'svg' | 'image' | 'video' | 'pdf' | 'diagram' | 'code' | 'binary';

/** Extensions with no useful inline rendering — the pane shows file meta +
 *  "open externally" for these. Anything not listed is treated as text/code
 *  (the backend's utf-8 check is the real arbiter). */
const BINARY_EXTS = new Set([
  'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'key', 'pages', 'numbers',
  'zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'dmg', 'iso', 'bin', 'exe', 'app',
  'mp3', 'wav', 'm4a', 'flac', 'mkv', 'avi', 'webm',
  'sqlite', 'db', 'ico', 'icns', 'ttf', 'otf', 'woff', 'woff2',
]);
const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp']);

export function chatArtifactKind(name: string): ChatArtifactKind {
  const m = /\.([a-z0-9]+)$/i.exec(name);
  const ext = (m?.[1] ?? '').toLowerCase();
  if (ext === 'html' || ext === 'htm') return 'html';
  if (ext === 'md' || ext === 'markdown' || ext === 'mdown' || ext === 'mkd') return 'markdown';
  if (ext === 'svg') return 'svg';
  if (ext === 'pdf') return 'pdf';
  if (ext === 'mmd' || ext === 'mermaid') return 'diagram';
  if (IMAGE_EXTS.has(ext)) return 'image';
  // mp4/m4v/mov play in the pane (mediaKinds.ts); webm, mkv and avi don't.
  if (isVideoPath(name)) return 'video';
  if (BINARY_EXTS.has(ext)) return 'binary';
  return 'code';
}
