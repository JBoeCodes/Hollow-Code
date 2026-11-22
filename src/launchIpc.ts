// Overnight Y7: the IPC calls this window makes while it launches, against a
// budget. A repeat of the same command with the same arguments is the waste
// five asks B4 found (the rail and the workspace page each ran the whole list
// chain), so the budget is on repeats; the total is reported for comparing
// builds. Imported by api.ts, which is on the launch path: keep it tiny and
// free of store imports.
export const LAUNCH_IPC_WINDOW_MS = 15_000;
/** The same command with the same arguments at most this often in the window
 *  (a refresh plus the follow-up after a write is two). */
export const LAUNCH_IPC_MAX_SAME_CALL = 2;

export interface LaunchIpcReport {
  /** Calls in the window so far; `settled` once the window is over. */
  total: number; commands: number; settled: boolean;
  /** Calls repeated past the budget, most repeated first: `command`, the
   *  arguments it was asked with (credentials masked, long strings cut) and how often. */
  repeated: { command: string; args: string; count: number }[];
}

const started = typeof performance === 'undefined' ? 0 : performance.now();
const calls = new Map<string, number>();
let total = 0;
let warned = false;

/** Long strings (a pasted image's base64, a note being saved) are cut before
 *  serializing, with their length kept, so tallying stays cheap and two calls
 *  that differ only past the cut still count apart. */
// Redact before the tally stores its key, not just when it renders. A secret
// is sensitive even when short; entire env/header maps may use arbitrary keys.
// Calls differing only in credentials deliberately share one diagnostic bucket.
const sensitive = /secret|token|password|passwd|credential|authorization|api.?key|private.?key|^(?:key|env|environment|headers?|cookie|bearer|auth|args)$/i;
// These commands' top-level key is a session identity, never a credential.
const sessionCommands = new Set(['session_target', 'read_agent_session', 'read_agent_conversation', 'read_artifact_session_context', 'archive_agent_session', 'unarchive_agent_session', 'rename_agent_session']);
const cut = (key: string, v: unknown) => sensitive.test(key) ? '[redacted]'
  : typeof v === 'string' && v.length > 64 ? `${v.slice(0, 48)}…${v.slice(-8)}#${v.length}` : v;
function argsKey(command: string, args: unknown): string {
  if (args === undefined) return '';
  try { return JSON.stringify(args, function (key, value) {
    if (this === args && key === 'key' && sessionCommands.has(command)) return value;
    return cut(key, value);
  }) ?? ''; } catch { return '?'; }
}

/** Count one call; after the window, warn once in the console when over budget. */
export function noteIpc(command: string, args?: unknown, now = performance.now()): void {
  if (now - started > LAUNCH_IPC_WINDOW_MS) {
    if (!warned) {
      warned = true;
      const over = launchIpcReport(now).repeated;
      if (over.length) console.warn('[hollow] launch IPC repeats over budget', over);
    }
    return;
  }
  total++;
  const key = `${command}\u0000${argsKey(command, args)}`;
  calls.set(key, (calls.get(key) ?? 0) + 1);
}

export function judgeLaunchIpc(counts: Map<string, number>, max = LAUNCH_IPC_MAX_SAME_CALL): LaunchIpcReport['repeated'] {
  return [...counts]
    .filter(([, n]) => n > max)
    .map(([key, count]) => { const [command, args = ''] = key.split('\u0000'); return { command, args, count }; })
    .sort((a, b) => b.count - a.count || a.command.localeCompare(b.command));
}

export function launchIpcReport(now = performance.now()): LaunchIpcReport {
  return {
    total,
    commands: new Set([...calls.keys()].map(k => k.split('\u0000')[0])).size,
    settled: now - started > LAUNCH_IPC_WINDOW_MS,
    repeated: judgeLaunchIpc(calls),
  };
}
