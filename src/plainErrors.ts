// Overnight C7: raw Rust, OS, network and parser errors in plain language.
// A message that carries one ("× approval failed: No such file or directory
// (os error 2)") keeps its own lead-in and gets one sentence saying what
// happened and what to do; the raw text moves to `detail`, which the
// snackbar shows under "Details". Messages already in plain language pass
// through untouched. Pure, so the node vitest suite can load it.
//
// A raw error counts only where one starts: at the start of the message or
// right after a ": " / " — " separator. A title the person chose ("Moved
// "database is locked.md" to Notion's trash") is never rewritten.

export interface PlainError {
  /** What the person reads. */
  text: string;
  /** The raw error, for the "Details" disclosure; null when nothing was
   *  translated. */
  detail: string | null;
  /** What kind of error it is, stable across wording changes (morning M6):
   *  match on this, never on the sentence. `errMsg` stays the raw text, so
   *  the prefixes callers already match on keep working. Null when nothing
   *  was translated. */
  code: ErrorCode | null;
}

/** The kinds `plainError` recognises (morning M6). */
export type ErrorCode =
  | 'disk-full' | 'read-only' | 'not-found' | 'not-allowed' | 'already-exists' | 'is-folder' | 'not-folder'
  | 'too-many-files' | 'busy' | 'pipe-closed' | 'db-busy' | 'db-damaged' | 'db-unopenable' | 'refused'
  | 'local-down' | 'offline' | 'signed-out' | 'service-error' | 'timed-out' | 'bad-data' | 'not-text'
  | 'no-command' | 'lock-failed' | 'crashed' | 'icon-damaged';

/** Each pattern must match where its segment starts (they are tried with
 *  `^`). First match wins, so specific patterns come before general ones. */
export const PLAIN_ERRORS: ReadonlyArray<readonly [RegExp, ErrorCode, string]> = [
  [/^(?:No space left on device|(?:(?!: | \u2014 ).)*\(os error 28\))/i, 'disk-full', 'The disk is full. Free some space, then try again.'],
  [/^(?:Read-only file system|(?:(?!: | \u2014 ).)*\(os error 30\))/i, 'read-only', 'That location is read-only, so Hollow can’t write there.'],
  [/^(?:No such file or directory|(?:(?!: | \u2014 ).)*\(os error 2\))/i, 'not-found', 'That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.'],
  [/^(?:Permission denied \(os error|Operation not permitted \(os error|(?:(?!: | \u2014 ).)*\(os error (?:1|13)\))/i, 'not-allowed', 'Hollow isn’t allowed to open that file or folder. Check its permissions (or macOS Privacy & Security), then try again.'],
  [/^(?:File exists \(os error|(?:(?!: | \u2014 ).)*\(os error 17\))/i, 'already-exists', 'Something with that name already exists. Pick another name.'],
  [/^(?:Is a directory \(os error|(?:(?!: | \u2014 ).)*\(os error 21\))/i, 'is-folder', 'That’s a folder, not a file.'],
  [/^(?:Not a directory \(os error|(?:(?!: | \u2014 ).)*\(os error 20\))/i, 'not-folder', 'Part of that path is a file, not a folder.'],
  [/^(?:Too many open files|(?:(?!: | \u2014 ).)*\(os error 24\))/i, 'too-many-files', 'Hollow has too many files open at once. Wait a moment and try again; restart Hollow if it keeps happening.'],
  [/^(?:Resource busy \(os error|Resource temporarily unavailable \(os error|(?:(?!: | \u2014 ).)*\(os error (?:16|35)\))/i, 'busy', 'That file or resource is busy. Close it in any other app, then try again.'],
  [/^(?:Broken pipe \(os error|(?:(?!: | \u2014 ).)*\(os error 32\))/i, 'pipe-closed', 'The connection to a helper process closed early. Try again.'],
  [/^(?:database is locked|database table is locked)/i, 'db-busy', 'Hollow’s database was busy. Wait a moment, then try again.'],
  [/^(?:database disk image is malformed|file is not a database)/i, 'db-damaged', 'A Hollow database file is damaged, so nothing was changed. Restart Hollow; if it keeps happening, use Save diagnostics… and share the file.'],
  [/^unable to open database file/i, 'db-unopenable', 'Hollow couldn’t open its database file. Check that the disk is connected and has space, then restart Hollow.'],
  [/^(?:Connection refused|(?:(?!: | \u2014 ).)*\(os error 61\))/i, 'refused', 'The service isn’t answering. Check that it’s running, then try again.'],
  [/^error sending request for url \(https?:\/\/(?:127\.0\.0\.1|localhost|\[::1\])[:/]/i, 'local-down', 'The local service isn’t answering. Check that it’s running, then try again.'],
  [/^(?:error sending request|dns error|failed to lookup address|Could not resolve host|Network is unreachable|(?:(?!: | \u2014 ).)*\(os error 51\))/i, 'offline', 'Couldn’t reach the network. Check your internet connection, then try again.'],
  [/^HTTP status client error \((?:401|403)\b/i, 'signed-out', 'The service turned the request down: the sign-in may have expired. Reconnect it in Settings, then try again.'],
  [/^HTTP status server error \(5\d\d\b/i, 'service-error', 'The service had a problem on its side. Try again in a few minutes.'],
  [/^(?:Operation timed out|deadline has elapsed|(?:(?!: | \u2014 ).)*\(os error 60\))/i, 'timed-out', 'It took too long and was stopped. Check your connection, then try again.'],
  [/^(?:(?:expected [^\n]*|EOF while parsing [^\n]*|invalid type: [^\n]*|missing field `[^`]+`|unknown variant `[^`]+`[^\n]*|trailing characters) at line \d+ column \d+|error decoding response body)/i, 'bad-data', 'Hollow couldn’t read the data it got back: it wasn’t in the expected format. Try again; if it keeps happening, it’s a bug worth reporting.'],
  [/^(?:stream did not contain valid UTF-8|invalid utf-8)/i, 'not-text', 'That isn’t plain text Hollow can read.'],
  [/^Command \S+ not found$/i, 'no-command', 'This version of Hollow doesn’t have that action. Restart Hollow to load the newest build.'],
  [/^(?:PoisonError|(?:(?!: | \u2014 ).)*lock poisoned|poisoned lock)/i, 'lock-failed', 'An internal lock failed after an earlier error. Restart Hollow.'],
  [/^(?:thread '[^']*' )?panicked at /i, 'crashed', 'Something inside Hollow crashed while doing that. Try again; if it repeats, restart Hollow.'],
];

/** Where segments start: the message's start and just after each ": " or
 *  " — " separator. */
function segmentStarts(message: string): number[] {
  const starts = [0];
  for (const m of message.matchAll(/: | — /g)) starts.push(m.index + m[0].length);
  return starts;
}

/** Noise a lead-in shouldn't end with ("Save failed: io error"). */
const NOISE_TAIL = /(?:(?:: | — )?(?:error|io error|fatal|caused by|Error))+$/i;

/** Hollow's own error codes, as the backend writes them (`TASK_CONFLICT: …`).
 *  The sentence after one is already plain; callers match the code on the raw
 *  message (`isTaskConflict`, `isSourceChanged`), so the person never has to
 *  read it (night ST10). Named one by one: an ALL_CAPS key the person typed
 *  (`GITHUB_TOKEN: …`) is not a code and stays. A new code goes here. */
export const HOLLOW_ERROR_CODES = [
  'BLOCK_CHANGED', 'CANVAS_CHANGED', 'CANVAS_INVALID', 'CANVAS_RETIRED', 'CHAT_CONTEXT_RECOVERED',
  'FEEDBACK_AMBIGUOUS_RUN', 'FEEDBACK_EXPORT_IN_VAULT', 'FEEDBACK_NO_TURN', 'FEEDBACK_TURN_RUNNING',
  'IDEMPOTENCY_MISMATCH', 'INTERACTION_CLOSED', 'ICON_CONFLICT', 'ICON_DAMAGED', 'PLAN_CHANGED',
  'RESULT_ACCEPTED_PERSON_ONLY', 'RESULT_ACCEPT_PERSON_ONLY', 'RESULT_CHANGED', 'REVIEW_CHANGED', 'REVIEW_INVALID',
  'SOURCE_CHANGED', 'TASK_BLOCKER', 'TASK_CONFLICT', 'TASK_CYCLE', 'TASK_INVALID', 'TASK_KEY', 'TASK_LINKED',
  'UNSAFE_WORKING_DIRECTORY', 'WORKFLOW_EDIT_CONFLICT', 'WORKTREE_UNSAVED', 'WRITE_CONFLICT',
] as const;
const HOLLOW_CODE = new RegExp(`(^|: | — )(?:${HOLLOW_ERROR_CODES.join('|')}): `, 'g');

/** `message` with its first raw error put in plain language. The text before
 *  the raw part ("Couldn't save: ") is kept as the lead-in; the raw part
 *  becomes the detail. A Hollow error code is dropped from the text, and the
 *  message as it came moves to the detail. */
export function plainError(message: string): PlainError {
  // The saved-record category owns even a nested parser failure. Keep the
  // original diagnostic for Details; the caller supplies its real Reset label.
  if (/(^|: | — )ICON_DAMAGED: /.test(message)) {
    return { text: 'The saved icon is damaged. Nothing was changed.', detail: message.trim(), code: 'icon-damaged' };
  }
  // Repeated: one match uses up the separator, so back-to-back codes need a second pass.
  let bare = message;
  for (let prev = ''; prev !== bare;) { prev = bare; bare = bare.replace(HOLLOW_CODE, '$1'); }
  for (const at of segmentStarts(bare)) {
    const raw = bare.slice(at);
    // One segment: up to the end, since a raw error's own detail may hold
    // separators (a URL's "://", "dns error: …").
    const hit = PLAIN_ERRORS.find(([pattern]) => pattern.test(raw.split('\n')[0]));
    if (!hit) continue;
    const lead = bare.slice(0, at).replace(/(?:: | — )$/, '').replace(NOISE_TAIL, '').trim();
    const text = lead ? `${lead}: ${hit[2]}` : hit[2];
    return { text, detail: bare === message ? raw.trim() : message.trim(), code: hit[1] };
  }
  return { text: bare, detail: bare === message ? null : message.trim(), code: null };
}

/** What an error line shows (overnight S5): `plainError` when `plain` (the
 *  default), else the text as it is. A note that is only sometimes an error
 *  passes `plain = false` when it isn't: `plainError` acts after every ": ",
 *  so a title the person chose ("Runbook “Deploy: Connection refused fix”
 *  started.") would otherwise read as a failure. */
export function shownError(text: string, plain = true): PlainError {
  return plain ? plainError(text) : { text, detail: null, code: null };
}

/** A failure the person can try again (overnight G2), as a snackbar: the
 *  message in plain language, the raw error under Details, and a Retry
 *  button that runs `retry`. Held 12 s so Retry can be reached. */
export function retryNotice(message: string, retry: () => void | Promise<void>) {
  const plain = plainError(message.replace(/^\s*×\s*/, ''));
  return {
    message: plain.text,
    ...(plain.detail ? { detail: plain.detail } : {}),
    kind: 'error' as const,
    actionLabel: 'Retry',
    undo: retry,
    ttlMs: 12000,
  };
}
