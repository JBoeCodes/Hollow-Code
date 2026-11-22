const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

function absoluteDate(ts: number): string {
  return new Date(ts).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
}

/** Compact elapsed/remaining time; dates older than seven days are absolute. */
export function formatRelative(ts: number, now: number): string {
  if (!Number.isFinite(ts) || !Number.isFinite(now)) return '';
  const elapsed = now - ts;
  const distance = Math.abs(elapsed);
  if (distance > 7 * DAY) return absoluteDate(ts);
  if (distance < MINUTE) return 'just now';
  const value = distance < HOUR ? `${Math.floor(distance / MINUTE)}m`
    : distance < DAY ? `${Math.floor(distance / HOUR)}h`
      : `${Math.floor(distance / DAY)}d`;
  return elapsed < 0 ? `in ${value}` : `${value} ago`;
}

/** Local clock today, weekday and clock within seven days, otherwise a date. */
export function formatClock(ts: number, now: number): string {
  if (!Number.isFinite(ts) || !Number.isFinite(now)) return '';
  const date = new Date(ts);
  const today = new Date(now);
  const clock = () => date.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit', hour12: true });
  if (date.getFullYear() === today.getFullYear() && date.getMonth() === today.getMonth()
    && date.getDate() === today.getDate()) return clock();
  if (Math.abs(now - ts) > 7 * DAY) return absoluteDate(ts);
  const weekday = date.toLocaleDateString('en-US', { weekday: 'short' });
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${weekday} ${month}-${day} ${clock()}`;
}

/** Measured duration: milliseconds, tenths of seconds, minutes or hours. */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms)) return '';
  const whole = Math.max(0, Math.round(ms));
  if (whole < 1_000) return `${whole}ms`;
  // Truncate tenths so a value just under a minute cannot read "60s".
  if (whole < MINUTE) return `${(Math.floor(whole / 100) / 10).toFixed(1).replace(/\.0$/, '')}s`;
  const seconds = Math.floor(whole / 1_000);
  if (whole < HOUR) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const minutes = Math.floor(seconds / 60);
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** Whole-second durations (history rows): `0s` rather than a false `0ms`. */
export function formatSeconds(sec: number): string {
  return Number.isFinite(sec) && sec <= 0 ? '0s' : formatDuration(sec * 1_000);
}
