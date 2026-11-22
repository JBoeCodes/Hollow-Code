// Parsing for the merged rows (Settings PR 5, plan 3.1 rule 2): one row can
// hold two inputs (heartbeat spacing and daily cap, check-in hours, chats at
// once overall and per workspace). Each input still saves on its own change;
// a value that isn't a whole number in its range is refused, never saved.

/** A whole number from `text` within `min` to `max`, or null. */
export function parseWhole(text: string, min: number, max: number): number | null {
  const t = text.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return n >= min && n <= max ? n : null;
}

