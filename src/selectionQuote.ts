/**
 * The Markdown quote for a passage selected in the transcript (overnight 11-07,
 * D1, audit gap 9). Pure, so the node suite can test it.
 *
 * Each line gets "> ", a blank line inside the selection stays as a bare ">"
 * (runs of them fold to one: a selection across blocks carries several), and
 * the quote ends with a blank line so the caret lands under it. A long
 * selection is cut at `QUOTE_LIMIT` characters with an ellipsis. The webview's
 * selections carry no-break spaces and CRLFs; both become plain.
 */
export const QUOTE_LIMIT = 4000;

/** Where to cut `text` at `limit` code units: one earlier when the cut would
 *  leave a lone high surrogate (the first half of an emoji) before the
 *  ellipsis. The bounds are written as numbers, 0xd800 to 0xdbff. */
function cutAt(text: string, limit: number): number {
  const last = text.charCodeAt(limit - 1);
  return last >= 0xd800 && last <= 0xdbff ? limit - 1 : limit;
}

export function quoteMarkdown(selected: string, limit = QUOTE_LIMIT): string {
  const text = selected.replace(/\r\n?/g, '\n').replace(/\xa0/g, ' ').trim();
  if (!text) return '';
  const cut = text.length > limit ? `${text.slice(0, cutAt(text, limit)).trimEnd()}…` : text;
  const lines: string[] = [];
  for (const raw of cut.split('\n')) {
    const line = raw.trimEnd();
    if (line === '' && lines[lines.length - 1] === '') continue;
    lines.push(line);
  }
  return `${lines.map(line => (line ? `> ${line}` : '>')).join('\n')}\n\n`;
}
