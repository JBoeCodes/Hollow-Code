/**
 * A Markdown document's outline (next-level P04/P08): its headings, skipping
 * fenced code and frontmatter. Pure, unit-tested.
 */
export interface OutlineItem { level: number; text: string; index: number }

export function markdownOutline(text: string): OutlineItem[] {
  const out: OutlineItem[] = [];
  let fence: string | null = null;
  let body = text;
  if (body.startsWith('---\n')) { const end = body.indexOf('\n---', 4); if (end > 0) body = body.slice(end + 4); }
  for (const raw of body.split('\n')) {
    const line = raw.trimEnd();
    const f = /^(\s*)(```|~~~)/.exec(line);
    if (f) { if (fence === f[2]) fence = null; else if (!fence) fence = f[2]; continue; }
    if (fence) continue;
    const h = /^(#{1,4})\s+(.+?)\s*#*$/.exec(line);
    if (h) out.push({ level: h[1].length, text: h[2].replace(/[*_`]/g, ''), index: out.length });
  }
  return out;
}
