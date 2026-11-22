/** A running conversation's elapsed time in a sidebar row: `45s` under a
 *  minute, `12m` under an hour, then `1h 05m`. Pure; the row's clock is
 *  `store/secondClock.ts`. */
export function formatRunElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '0s';
  const total = Math.floor(ms / 1000);
  if (total < 60) return `${total}s`;
  const minutes = Math.floor(total / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, '0')}m`;
}
