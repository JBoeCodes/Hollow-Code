export type TranscriptMatchPart = { text: string; match: boolean };

/** Same trim/lowercase/literal semantics as transcript filtering; never HTML. */
export function transcriptMatchParts(text: string, query: string): TranscriptMatchPart[] {
  const q = query.trim().toLowerCase();
  const lower = text.toLowerCase();
  if (!q || !lower.includes(q)) return [{ text, match: false }];
  // A lowercase expansion (İ -> i + combining dot) must not shift the
  // highlight or split the original character. Most text needs no map.
  const map: Array<{ start: number; end: number }> = [];
  if (lower.length !== text.length) {
    let start = 0;
    for (const char of text) {
      const end = start + char.length;
      for (let i = 0; i < char.toLowerCase().length; i++) map.push({ start, end });
      start = end;
    }
  }
  const parts: TranscriptMatchPart[] = [];
  let cursor = 0;
  let at = lower.indexOf(q);
  while (at >= 0) {
    const start = map.length ? map[at].start : at;
    const end = map.length ? map[at + q.length - 1].end : at + q.length;
    if (end > cursor) {
      if (start > cursor) parts.push({ text: text.slice(cursor, start), match: false });
      parts.push({ text: text.slice(Math.max(cursor, start), end), match: true });
      cursor = end;
    }
    at = lower.indexOf(q, at + q.length);
  }
  if (cursor < text.length) parts.push({ text: text.slice(cursor), match: false });
  return parts;
}
