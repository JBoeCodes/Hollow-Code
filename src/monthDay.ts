// "Oct 14" in the person's locale (month short, day numeric), from one
// Intl.DateTimeFormat (ST2 follow-up, 2026-10-14). `toLocaleDateString` with
// options builds a new formatter on every call, and these labels are drawn
// per row: due dates in Tasks, the sidebar's and Home's "when", imported
// conversations. Same output as that call; an invalid date keeps its own text
// ("Invalid Date"), where the formatter would throw. The formatter keeps the
// time zone of its first use until a reload.
const OPTIONS: Intl.DateTimeFormatOptions = { month: 'short', day: 'numeric' };
let format: Intl.DateTimeFormat | null = null;

export function monthDay(date: Date): string {
  if (Number.isNaN(date.getTime())) return date.toLocaleDateString(undefined, OPTIONS);
  format ??= new Intl.DateTimeFormat(undefined, OPTIONS);
  return format.format(date);
}
