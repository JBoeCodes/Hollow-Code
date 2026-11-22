import { plainError } from './plainErrors';
import { errMsg } from './utils';

/** Why a file couldn't be shown, as the Files view words it in place of the
 *  file (10-23: a failed read used to take the whole page down). `kind`
 *  picks the way on; `detail` is the raw error for Details, null when the
 *  sentence already says everything. */
export interface PreviewFailure {
  kind: 'missing' | 'folder' | 'too-large' | 'other';
  text: string;
  detail: string | null;
}

/** `chat_preview_file`'s refusals (chat::preview_file) and the OS errors
 *  behind them, in plain language. Pure, so the node suite can load it. */
export function previewFailure(error: unknown): PreviewFailure {
  const raw = errMsg(error).trim();
  if (/^not a file: /i.test(raw)) return { kind: 'folder', text: 'This is a folder, not a file, so there’s nothing to preview. Show it in Finder to see what’s inside.', detail: raw };
  // The size, or (when the read stopped at the cap) only the limit: the
  // backend says "(12 MB)" or "(limit 10 MB)" (2026-11-04: the second showed
  // as a generic error).
  const big = /^image too large to preview \((limit )?(\d+) MB\)/i.exec(raw);
  if (big) return { kind: 'too-large', text: big[1] ? `This picture is over ${big[2]} MB, too large to preview here. Open it in its own app instead.` : `This picture is ${big[2]} MB, too large to preview here (the limit is 10 MB). Open it in its own app instead.`, detail: null };
  if (/^preview path must be absolute/i.test(raw)) return { kind: 'missing', text: 'Hollow couldn’t tell which folder this path is in, so it can’t find the file.', detail: raw };
  const plain = plainError(raw);
  if (plain.code === 'not-found') return { kind: 'missing', text: 'It isn’t at this path: it may have been moved or deleted, or the reply named it relative to another folder.', detail: plain.detail ?? raw };
  if (plain.code === 'is-folder') return { kind: 'folder', text: 'This is a folder, not a file, so there’s nothing to preview. Show it in Finder to see what’s inside.', detail: plain.detail ?? raw };
  return { kind: 'other', text: plain.text || 'Hollow couldn’t read it.', detail: plain.detail };
}
