/**
 * Pure helpers for the connected-project file list and reader (next-level
 * P07). The list is paged from `connected_files`; these decide how a file's
 * state reads, and where a reader opened from a search lands.
 */
import type { ConnectedFile, ConnectedFileState } from './types';

const DOC_KINDS: Record<string, string> = { pdf: 'PDF', docx: 'Word', pptx: 'PowerPoint', xlsx: 'Excel' };

/** The tag a listed file wears, or null for ordinary readable text. */
export function connectedFileTag(f: Pick<ConnectedFile, 'state' | 'readable'> & { relative?: string }): { label: string; hint: string } | null {
  const state: ConnectedFileState = f.state ?? (f.readable ? 'text' : 'needsReader');
  if (state === 'tooLarge') return { label: 'too large · not read', hint: 'Larger than the 1 MB text limit. Hollow lists it but never reads it as empty: open the original, or export a smaller text file.' };
  if (state === 'needsReader') return { label: 'needs reader', hint: 'Needs a reader or a text export before agents can use it' };
  if (state === 'extracted') return { label: `read by Hollow · ${DOC_KINDS[(f.relative ?? '').split('.').pop()?.toLowerCase() ?? ''] ?? 'document'}`, hint: 'Hollow reads the text of this PDF or Office file itself, so agents can cite it. A scan with no text layer can’t be read.' };
  return null;
}

export const FILE_STATE_OPTIONS: readonly { value: '' | ConnectedFileState; label: string }[] = [
  { value: '', label: 'All files' },
  { value: 'text', label: 'Readable text' },
  { value: 'extracted', label: 'Read by Hollow (PDF, Office)' },
  { value: 'tooLarge', label: 'Too large to read' },
  { value: 'needsReader', label: 'Needs a reader' },
];

/**
 * Split `text` around the first line holding the most search words, so a
 * reader opened from a search shows (and scrolls to) the relevant content.
 * Null when no word of the query occurs.
 */
export function findLine(text: string, query: string): { before: string; line: string; after: string; number: number } | null {
  const words = query.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(w => w.length > 1);
  if (!words.length || !text) return null;
  const lines = text.split('\n');
  let best = -1, bestHits = 0;
  lines.forEach((l, i) => {
    const lower = l.toLowerCase();
    const hits = words.filter(w => lower.includes(w)).length;
    if (hits > bestHits) { best = i; bestHits = hits; }
  });
  if (best < 0) return null;
  const before = lines.slice(0, best).join('\n');
  return { before: before ? `${before}\n` : '', line: lines[best], after: best < lines.length - 1 ? `\n${lines.slice(best + 1).join('\n')}` : '', number: best + 1 };
}

const n = (x: number) => x.toLocaleString('en-US');

/** The list's count line: what is shown out of what matched. */
export function listCountText(shown: number, total: number, direct: number, query: string): string {
  const found = direct ? ` · ${n(direct)} more found directly` : '';
  if (!total) return query ? `No listed file matches${found}` : 'No files';
  return shown < total ? `Showing ${n(shown)} of ${n(total)} files${found}` : `${n(total)} ${total === 1 ? 'file' : 'files'}${found}`;
}
