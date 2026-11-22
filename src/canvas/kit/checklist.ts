/** Plain checklist rows; a mark is evidence only when explicitly authored.
 * Repeated labels keep their first value and are disclosed by the component. */
export function checklistRows(lines: readonly string[]): { rows: { label: string; done: boolean }[]; repeated: number } {
  const rows: { label: string; done: boolean }[] = [], seen = new Set<string>();
  let repeated = 0;
  for (const raw of lines) {
    const text = raw.trim().replace(/^[-*]\s+/, '');
    const mark = /^\[([ xX])\](?:\s+|$)/.exec(text);
    const label = (mark ? text.slice(mark[0].length) : text).trim();
    if (!label) continue;
    if (seen.has(label)) { repeated++; continue; }
    seen.add(label);
    rows.push({ label, done: !!mark && mark[1].toLowerCase() === 'x' });
  }
  return { rows, repeated };
}
