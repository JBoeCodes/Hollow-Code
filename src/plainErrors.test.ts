import { describe, expect, it } from 'vitest';
import { PLAIN_ERRORS, plainError, shownError } from './plainErrors';
import SNACKBAR_STATE from './store/snackbars.ts?raw';
import SNACKBARS from './components/SnackbarStack.tsx?raw';
import UI from './components/ui.tsx?raw';

// What the person reads for each raw error, lead-in kept (overnight C7).
const CASES: Array<[string, string, string | null]> = [
  ['× approval failed: No such file or directory (os error 2)',
    '× approval failed: That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.',
    'No such file or directory (os error 2)'],
  ['Couldn’t save /Users/x/a.md: Permission denied (os error 13)',
    'Couldn’t save /Users/x/a.md: Hollow isn’t allowed to open that file or folder. Check its permissions (or macOS Privacy & Security), then try again.',
    'Permission denied (os error 13)'],
  ['write failed: No space left on device (os error 28)', 'write failed: The disk is full. Free some space, then try again.', 'No space left on device (os error 28)'],
  ['database is locked', 'Hollow’s database was busy. Wait a moment, then try again.', 'database is locked'],
  ['× mark read failed: error sending request for url (https://api.notion.com/v1/pages): dns error: failed to lookup address information',
    '× mark read failed: Couldn’t reach the network. Check your internet connection, then try again.',
    'error sending request for url (https://api.notion.com/v1/pages): dns error: failed to lookup address information'],
  ['Couldn’t load the plan: expected value at line 1 column 1',
    'Couldn’t load the plan: Hollow couldn’t read the data it got back: it wasn’t in the expected format. Try again; if it keeps happening, it’s a bug worth reporting.',
    'expected value at line 1 column 1'],
  ['Command chat_send_x not found', 'This version of Hollow doesn’t have that action. Restart Hollow to load the newest build.', 'Command chat_send_x not found'],
  ['Connect failed — Connection refused (os error 61)', 'Connect failed: The service isn’t answering. Check that it’s running, then try again.', 'Connection refused (os error 61)'],
  ['Save failed: io error: Permission denied (os error 13)', 'Save failed: Hollow isn’t allowed to open that file or folder. Check its permissions (or macOS Privacy & Security), then try again.', 'Permission denied (os error 13)'],
  ['Couldn’t stop: Error: No such file or directory (os error 2)', 'Couldn’t stop: That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.', 'No such file or directory (os error 2)'],
  ['× Jev failed: error sending request for url (http://127.0.0.1:4315/v1/chat)', '× Jev failed: The local service isn’t answering. Check that it’s running, then try again.', 'error sending request for url (http://127.0.0.1:4315/v1/chat)'],
  ['Notion sync: HTTP status client error (401 Unauthorized) for url (https://api.notion.com/v1/search)', 'Notion sync: The service turned the request down: the sign-in may have expired. Reconnect it in Settings, then try again.', 'HTTP status client error (401 Unauthorized) for url (https://api.notion.com/v1/search)'],
  ['thread \'tokio-runtime-worker\' panicked at src/chat.rs:12:5', 'Something inside Hollow crashed while doing that. Try again; if it repeats, restart Hollow.', 'thread \'tokio-runtime-worker\' panicked at src/chat.rs:12:5'],
  ['rename failed: Read-only file system (os error 30)', 'rename failed: That location is read-only, so Hollow can’t write there.', 'Read-only file system (os error 30)'],
];

describe('plainError', () => {
  it('keeps damaged-icon recovery distinct from an embedded parser failure (P1-56)', () => {
    const raw = 'ICON_DAMAGED: Saved icon is damaged; left unchanged: EOF while parsing a value at line 1 column 1';
    const shown = plainError(raw);
    expect(shown.text).toMatch(/saved icon.*damaged/i);
    expect(shown.text).not.toContain('ICON_DAMAGED');
    expect(shown.text).not.toMatch(/try again/i);
    expect(shown.detail).toBe(raw);
  });

  it.each(CASES)('%s', (raw, text, detail) => {
    expect(plainError(raw)).toMatchObject({ text, detail });
  });

  it('leaves plain messages alone, including ones that merely mention a timeout or permission', () => {
    for (const msg of [
      'Saved',
      '× Couldn’t move it: a task with that name is already there.',
      'Codex timed out after 30 min without output (the watchdog stopped it).',
      'Permission denied by the person on the card.',
      'File exists in the vault; pick another name.',
      // Titles and paths the person chose, inside success messages.
      'Moved to Fix ENOENT on startup',
      'Moved "database is locked.md" to Notion’s trash',
      'Accepted "Investigate thread panicked at boot"',
      'Kept a copy in Notes/expected value at line 1 column 2.md',
      'The command claude not found in PATH, so Claude is off',
    ]) {
      expect(plainError(msg)).toEqual({ text: msg, detail: null, code: null });
    }
  });

  it('leaves the Work banner’s own sentences as they are (A08)', () => {
    // The banner now puts every error through plainError; Hollow's own lines must come out unchanged.
    for (const msg of [
      'Stop the current agent turn before removing its worktree.',
      'Stop the current agent turn before archiving this conversation.',
      'Could not copy: NotAllowedError: Write permission denied.',
    ]) {
      expect(plainError(msg)).toEqual({ text: msg, detail: null, code: null });
    }
  });

  it('drops a Hollow error code from what the person reads and keeps the message under Details (night ST10)', () => {
    expect(plainError('WRITE_CONFLICT: Notes/a.md changed since you opened it, so nothing was saved.')).toEqual({
      text: 'Notes/a.md changed since you opened it, so nothing was saved.',
      detail: 'WRITE_CONFLICT: Notes/a.md changed since you opened it, so nothing was saved.',
      code: null,
    });
    expect(plainError('× Couldn’t save: TASK_CONFLICT: tasks/x.md changed on disk after it was opened.').text)
      .toBe('× Couldn’t save: tasks/x.md changed on disk after it was opened.');
    expect(plainError('Error: RESULT_CHANGED: the file changed').text).toBe('Error: the file changed');
    expect(plainError('Couldn’t save: TASK_CONFLICT: WRITE_CONFLICT: x changed')).toEqual({
      text: 'Couldn’t save: x changed', detail: 'Couldn’t save: TASK_CONFLICT: WRITE_CONFLICT: x changed', code: null,
    });
    expect(plainError('TASK_CONFLICT: WRITE_CONFLICT: x').text).toBe('x');
    // A code ahead of a raw OS error: both go; the detail is the message as it came.
    expect(plainError('Save failed: WRITE_CONFLICT: No such file or directory (os error 2)')).toMatchObject({
      text: 'Save failed: That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.',
      detail: 'Save failed: WRITE_CONFLICT: No such file or directory (os error 2)',
      code: 'not-found',
    });
    expect(plainError('TASK_LINKED: edit it in Notion').detail).not.toBeNull();
    // Not a code: an env var in a sentence, a code mid-sentence, a single word, a key the person typed.
    for (const msg of ['Set HOLLOW_DEBUG_LOG=1 at launch.', 'It said TASK_CONFLICT: twice', 'NOTE: read this', '× GITHUB_TOKEN: use a single-line value']) {
      expect(plainError(msg)).toEqual({ text: msg, detail: null, code: null });
    }
  });

  it('translates the earliest raw error and keeps the rest in the detail', () => {
    // A raw error with its own separators (a URL, "dns error: …") stays whole.
    const { text, detail } = plainError('copy failed: No such file or directory (os error 2); cleanup: Permission denied (os error 13)');
    expect(text.startsWith('copy failed: That file or folder')).toBe(true);
    expect(detail).toBe('No such file or directory (os error 2); cleanup: Permission denied (os error 13)');
  });

  it('every sentence says what to do or what happened, in one or two sentences, without jargon', () => {
    for (const [, , sentence] of PLAIN_ERRORS) {
      expect(sentence).toMatch(/[.!]$/);
      expect(sentence).not.toMatch(/os error|errno|EOF|stderr|panick|SQLITE|UTF-8/);
      expect(sentence.split('. ').length).toBeLessThanOrEqual(3);
    }
  });
});

describe('where it shows', () => {
  it('flashFooter translates, marks a raw error as a failure and keeps the raw text as detail', () => {
    expect(SNACKBAR_STATE).toContain("const plain = plainError(msg.replace(/^\\s*×\\s*/, ''));");
    expect(SNACKBAR_STATE).toContain('const failed = /^\\s*×/.test(msg) || plain.detail !== null;');
    expect(SNACKBAR_STATE).toContain('...(plain.detail ? { detail: plain.detail } : {}),');
    // A translated error is a failure, so it gets the 7 s floor (P0-26).
    expect(SNACKBAR_STATE).toContain('ttlMs: failed ? Math.max(holdMs ?? 0, 7000) : holdMs ?? 3000,');
  });

  it('the snackbar shows the raw error under a Details disclosure', () => {
    expect(SNACKBARS).toContain('<details class="snackbar-details">');
    expect(SNACKBARS).toContain('<summary>Details</summary>');
  });
});

describe('error codes (morning M6)', () => {
  it('names each kind once, stable and plain', () => {
    const codes = PLAIN_ERRORS.map(([, code]) => code);
    expect(new Set(codes).size).toBe(codes.length);
    for (const code of codes) expect(code).toMatch(/^[a-z]+(?:-[a-z]+)*$/);
  });
  it('gives callers a code to match on instead of the wording', () => {
    expect(plainError('× approval failed: No such file or directory (os error 2)').code).toBe('not-found');
    expect(plainError('database is locked').code).toBe('db-busy');
    expect(plainError('Notion sync: HTTP status client error (401 Unauthorized) for url (x)').code).toBe('signed-out');
    expect(plainError('write failed: No space left on device (os error 28)').code).toBe('disk-full');
    expect(plainError('Saved').code).toBeNull();
    expect(plainError('Operation timed out').code).toBe('timed-out');
  });
});

describe('inline errors in plain language (morning M6)', () => {
  const sources = import.meta.glob('./**/*.tsx', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

  it('ErrorText shows the plain sentence and keeps the raw error under Details', () => {
    expect(UI).toContain('const plain = createMemo(() => shownError(props.text, props.plain !== false));');
    expect(UI).toContain('aria-expanded={open()} aria-controls={id} onClick={() => setOpen(o => !o)}>Details</button>');
    expect(UI).toContain('<code class="inline-error-detail" id={id} hidden={!open()}>{d()}</code>');
    expect(UI).toContain('const id = createUniqueId();');
    // Inside the alert, never a sibling of it: a <details> can't sit in a <p>.
    expect(UI).not.toMatch(/<details class="inline-error/);
  });

  it('no inline error prints the raw message straight into an alert', () => {
    const raw = /<Show when=\{(\w+)\(\)\}><(p|div) (?:class="[\w-]+" role="alert"|role="alert" class="[\w-]+")>\{\1\(\)\}<\/\2><\/Show>/;
    const offenders = Object.entries(sources).filter(([, s]) => raw.test(s)).map(([f]) => f);
    expect(offenders).toEqual([]);
    const using = Object.entries(sources).filter(([f, s]) => !f.endsWith('/ui.tsx') && s.includes('<ErrorText ')).length;
    expect(using).toBeGreaterThanOrEqual(40);
  });

  // Overnight S5: the lines M6 left, built some other way. A raw error
  // interpolated as the first thing after an alert's own lead-in ("Focus
  // blocks couldn’t be loaded: {errMsg(e)}"), as a failed StatePanel's body
  // (pass it as `error={…}` instead), or as `plainError(…).text` (the
  // sentence without its Details); and a note whose role turns to alert on
  // failure printing its text as it is.
  const RAW = String.raw`\{(?:errMsg\([^)]*\)|String\([^)]*\)|\w*[eE]rror\(\)|[\w.()!]*\.(?:error|lastError)|plainError\([^}]*\.text)\}`;
  const inAlert = new RegExp(String.raw`role="alert"[^>]*>[^<]*` + RAW);
  const inPanel = new RegExp('>' + RAW + String.raw`[^<{]*</StatePanel>`);
  const inNote = new RegExp(String.raw`role=\{[^}]*'alert'[^}]*\}>[^<]*(?:` + RAW + String.raw`|\{\w+\(\)!?(?:\.text)?\})`);
  // Raw on purpose, or left for now. Each needs a reason, and each must
  // still trip the scan (so a fixed file leaves the list).
  const LEFT: Record<string, string> = {
    // The self-build's own failure: a build log line, for the developer.
    './components/AppBuildPanel.tsx': 'build log',
    // Another lane's files on 10-13 (question cards; the editor with inline
    // AI). Route them through ErrorText / PlainErrorText next.
    './components/QuestionCard.tsx': 'other lane',
    './components/WorkFileEditor.tsx': 'other lane',
  };
  const trips = (s: string) => inAlert.test(s) || inPanel.test(s) || inNote.test(s);

  it('no alert line, failed panel or failing note shows a raw error (overnight S5)', () => {
    const offenders = Object.entries(sources).filter(([f, s]) => !(f in LEFT) && trips(s)).map(([f]) => f);
    expect(offenders).toEqual([]);
    for (const f of Object.keys(LEFT)) expect(sources[f] && trips(sources[f]), `${f} no longer needs its LEFT entry`).toBe(true);
    // The scan still sees what it is meant to see, and passes the fixed shapes.
    expect(inAlert.test('<p class="x" role="alert">Couldn’t load: {errMsg(list.error)}</p>')).toBe(true);
    expect(inAlert.test('<p role="alert" class="x">{loadError()}</p>')).toBe(true);
    expect(inAlert.test('<p class="x" role="alert">{plainError(`a: ${errMsg(e)}`).text}</p>')).toBe(true);
    expect(inPanel.test('<StatePanel kind="failed" title="t">{String(brief.error)}</StatePanel>')).toBe(true);
    expect(inPanel.test('<StatePanel kind="failed" title="t" error={brief.error} />')).toBe(false);
    expect(inPanel.test('<StatePanel kind="failed" title="t" error={e}>{\' \'}The rest still shows.</StatePanel>')).toBe(false);
    expect(inNote.test(`<p role={failed() ? 'alert' : 'status'}>{message()}</p>`)).toBe(true);
    expect(inNote.test(`<p role={n().error ? 'alert' : 'status'}>{n().text}</p>`)).toBe(true);
    expect(inNote.test(`<p role={n().error ? 'alert' : 'status'}><PlainErrorText text={n().text} plain={n().error} /></p>`)).toBe(false);
  });

  // Overnight A09: six lines the S5 scan can't see, because none of them sat
  // in a `role="alert"` element: a lead-in followed by the raw signal as
  // text, a Plaud test line that put the raw error next to ✓ outcomes, the
  // Claude accounts notes, and a skill body that carried "(read failed: …)"
  // into the Markdown. Each file names the raw shapes it must not go back to;
  // `${…}` (a template handing the text to ErrorText) and `={…}` (a prop)
  // are fine.
  const RAW_LINES: Record<string, RegExp[]> = {
    './MdPreview.tsx': [/(?<![$=])\{error\(\)\}/],
    './components/ExecutionReceipt.tsx': [/(?<![$=])\{patchError\(\)\}/],
    // Quick ingest's picker error moved with its sheet into Add (S5, 2026-11-03).
    './modals/AddModal.tsx': [/(?<![$=])\{error\(\)!?\}/],
    // `× ${…}` in a template, or `'× ' + …` by concatenation (A09 review).
    './settings/ConnectorRows.tsx': [/\{testMsg\(\) \?\?/, /[`'"]×\s?(?:\$\{|[^`'"\n]*[`'"]\s*\+)/],
    './settings/ClaudeAccountsPanel.tsx': [/class="[^"]*\bclaude-acct-error\b[^"]*">\{\w+\(\)\}/],
    './pages/Skills.tsx': [/read failed:/],
  };

  it('six more inline error lines speak plainly, with the raw error under Details (overnight A09)', () => {
    const offenders = Object.entries(RAW_LINES).flatMap(([f, shapes]) => {
      expect(sources[f], `${f} is missing; move its entry`).toBeDefined();
      return shapes.filter(shape => shape.test(sources[f])).map(shape => `${f} ${shape}`);
    });
    expect(offenders).toEqual([]);
    for (const f of Object.keys(RAW_LINES)) expect(sources[f], f).toMatch(/<(?:Plain)?ErrorText /);
    // The shapes main had trip the scan; the fixed ones don't.
    const [preview] = RAW_LINES['./MdPreview.tsx'];
    expect(preview.test('<div class="md-preview-error">Couldn\'t open this file: {error()}</div>')).toBe(true);
    expect(preview.test('<Show when={error()}><ErrorText class="md-preview-error" as="div" text={`Couldn\'t open this file: ${error()}`} /></Show>')).toBe(false);
    const [picker] = RAW_LINES['./modals/AddModal.tsx'];
    expect(picker.test('<p class="warn">{error()}</p>')).toBe(true);
    expect(picker.test('<Show when={error()}>{msg => <ErrorText class="work-error" text={msg()} />}</Show>')).toBe(false);
    const [plaudLine, plaudSet] = RAW_LINES['./settings/ConnectorRows.tsx'];
    expect(plaudLine.test('<div class="settings-row-warn">{testMsg() ?? plaudMessage()}</div>')).toBe(true);
    expect(plaudSet.test('setTestMsg(`× ${e instanceof Error ? e.message : String(e)}`);')).toBe(true);
    expect(plaudSet.test('setTestMsg(ok ? \'✓ Plaud accepted the token\' : \'× Plaud rejected the token\');')).toBe(false);
    expect(plaudSet.test("setTestMsg('× ' + errMsg(e));")).toBe(true);
    expect(plaudSet.test('setTestMsg("× " +String(e));')).toBe(true);
    expect(plaudSet.test("setTestMsg('×' + e);")).toBe(true);
    expect(plaudSet.test("setTestMsg('× Failed: ' + e);")).toBe(true);
    const [accounts] = RAW_LINES['./settings/ClaudeAccountsPanel.tsx'];
    expect(accounts.test('{msg => <p class="settings-note claude-acct-error">{msg()}</p>}')).toBe(true);
    expect(accounts.test('{msg => <ErrorText class="settings-note claude-acct-error" text={msg()} />}')).toBe(false);
  });

  it('a note that is only sometimes an error is translated only when it is one', () => {
    // plainError acts after every ": ", so a title the person chose would read as a failure.
    const started = 'Runbook “Deploy: Connection refused fix” started.';
    expect(plainError(started).text).not.toBe(started);
    expect(shownError(started, false)).toEqual({ text: started, detail: null, code: null });
    expect(shownError('Couldn’t start it: Connection refused (os error 61)').code).toBe('refused');
    // Every such note passes its error flag, whether its role is conditional or always status.
    const unflagged = /(?:role=\{[^}]*'alert'[^}]*\}|role="status"[^>]*)><PlainErrorText (?![^>]*\bplain=)/;
    expect(Object.entries(sources).filter(([, s]) => unflagged.test(s)).map(([f]) => f)).toEqual([]);
    expect(unflagged.test(`<p role={n().error ? 'alert' : 'status'}><PlainErrorText text={n().text} /></p>`)).toBe(true);
  });

  it('PlainErrorText is the same sentence and Details, without an element of its own', () => {
    expect(UI).toContain('export function PlainErrorText(props: { text: string; plain?: boolean; children?: JSX.Element })');
    expect(UI).toContain('return <>{plain().text}{props.children}<ErrorDetails detail={plain().detail} /></>;');
    // A failed StatePanel's `error`: the sentence, its children, then Details.
    expect(UI).toContain('<PlainErrorText text={errMsg(props.error)}>{props.children}</PlainErrorText>');
    const using = Object.entries(sources).filter(([f, s]) => !f.endsWith('/ui.tsx') && s.includes('<PlainErrorText ')).length;
    // 18 since COO rebuild L4 removed the workspace sections that used it.
    expect(using).toBeGreaterThanOrEqual(17);
  });
});
