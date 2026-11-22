/** Newest-first loaded history, bounded to RunHistory's reading list.
 * Missing/invalid durations break the line rather than becoming zero. */
export function runDurations(entries: readonly { durationSec?: number | null }[]) {
  const values = entries.slice(0, 25).reverse().map(entry =>
    typeof entry.durationSec === 'number' && Number.isFinite(entry.durationSec) && entry.durationSec >= 0
      ? entry.durationSec : null);
  const max = Math.max(0, ...values.filter((n): n is number => n !== null));
  const points = values.map((seconds, i) => ({ seconds,
    x: values.length === 1 ? 150 : 4 + i * 292 / Math.max(1, values.length - 1),
    y: seconds === null ? 44 : 44 - seconds / (max || 1) * 36,
  }));
  let connected = false;
  const path = points.map(p => {
    if (p.seconds === null) { connected = false; return ''; }
    const segment = `${connected ? 'L' : 'M'}${p.x},${p.y}`;
    connected = true;
    return segment;
  }).join('');
  return { points, max, path };
}
