// Export a Result as Markdown (overnight F6): which Results can go, the file
// name offered, and what the person reads after. Pure; the dialog and call
// are in store/resultExport.ts, the writing in result_export.rs.

/** Text Results export (Markdown as is, the rest fenced); binaries don't. */
const TEXT = ['md', 'markdown', 'txt', 'csv', 'tsv', 'json', 'yaml', 'yml', 'html'];

export function canExportMarkdown(name: string): boolean {
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1).toLowerCase() : '';
  return TEXT.includes(ext);
}

/** `Q3 plan: draft/v2` → `Q3 plan- draft-v2.md`: the title, made safe. */
export function exportFileName(title: string): string {
  const safe = [...title.replace(/[/\\:*?"<>|\u0000-\u001f]+/g, '-').replace(/\s+/g, ' ').replace(/^[\s.]+/, '')]
    .slice(0, 80).join('').trim();
  return `${safe || 'Result'}.md`;
}

export function exportedText(target: string, sources: number): string {
  const listed = sources ? `, with ${sources} source${sources === 1 ? '' : 's'} listed` : '';
  return `Saved as Markdown to ${target}${listed}. Nothing was sent.`;
}
