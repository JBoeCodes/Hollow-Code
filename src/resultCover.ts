/** A document cover describes listed metadata; it does not preview file contents. */
export function resultCover(card: { name: string; kindLabel: string; size: number }): { format: string; size: string | null } {
  const name = card.name.split('/').pop() ?? '';
  const dot = name.lastIndexOf('.');
  const ext = dot > 0 ? name.slice(dot + 1) : '';
  const format = /^[a-z0-9]{1,8}$/i.test(ext) ? ext.toUpperCase() : card.kindLabel;
  const bytes = card.size;
  const size = !Number.isFinite(bytes) || bytes < 0 ? null : bytes < 1024 ? `${bytes} B`
    : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return { format, size };
}
