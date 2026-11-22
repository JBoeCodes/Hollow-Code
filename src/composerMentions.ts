/** Completion ranges are caret-local: mail addresses and shell variables in
 * the middle of a word do not open a picker. Text outside the range survives. */
export interface MentionRange { kind: '@' | '$'; query: string; from: number; to: number }
export function mentionAt(text: string, caret: number): MentionRange | null {
  const before = text.slice(0, caret);
  const match = /(?:^|\s)([@$])([^\s@$]*)$/.exec(before);
  return match ? { kind: match[1] as '@' | '$', query: match[2], from: caret - match[2].length - 1, to: caret } : null;
}
export function replaceMention(text: string, range: MentionRange, insert: string): { text: string; caret: number } {
  return { text: text.slice(0, range.from) + insert + ' ' + text.slice(range.to), caret: range.from + insert.length + 1 };
}
export function fileMention(name: string, path: string): string {
  const label = name.replace(/[\\[\]]/g, '\\$&');
  return `[@${label}](<${path.split('/').map(encodeURIComponent).join('/')}>)`;
}
export function mentionedFiles(text: string): { name: string; path: string; from: number; to: number }[] {
  const files = [];
  const pattern = /\[@((?:\\.|[^\]\\])*)\]\(<([^>\n]+)>\)/g;
  for (const match of text.matchAll(pattern)) {
    try {
      files.push({ name: match[1].replace(/\\(.)/g, '$1'), path: decodeURIComponent(match[2]), from: match.index!, to: match.index! + match[0].length });
    } catch { /* Incomplete manual edits remain ordinary text. */ }
  }
  return files;
}
export interface MentionChoice { name: string; desc: string; source: 'file' | 'skill'; insert: string }
