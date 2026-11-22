#!/usr/bin/env node
// No engineering words in what the app says to the person (night ST10,
// 2026-10-14).
//
//   node scripts/check-plain-words.mjs        (also run by npm run test:scripts)
//
// Reads every sentence the frontend can show (JSX text, and string or
// template literals that read as a sentence: two words or more, starting with
// a capital or a digit, or placed straight into JSX as content, where a
// lower-case fragment such as `; changes are in the run record` shows too, or
// the fallback of `||` / `??`, shown in place of a missing value (P1-03); not
// inside <style> or <script>) and fails on a word from PLAIN_WORDS. Not
// read: imports, class/id/data/aria-reference attributes, comparisons, switch
// cases, types, and arguments to calls that never show text (invoke, listen,
// querySelector, console…). Tests, mocks, the smoke harness, the canvas kit
// and generated files are skipped.
//
// The words and what to say instead are in WORDS, which also writes the
// glossary's "Words we use" table (`node scripts/check-plain-words.mjs
// --glossary`). A sentence written for an agent rather than the person goes in
// ALLOWED with its reason.
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const root = fileURLToPath(new URL('..', import.meta.url));

/**
 * The person-facing words (P1-03): one list feeds PLAIN_WORDS and the "Words
 * we use" table in docs/reference/glossary.md (`--glossary` rewrites that
 * table; check-plain-words.test.mjs fails when the two disagree). A row with
 * `ban` is an engineering word this script refuses in shown text. The other
 * rows settle a collision, one meaning per word: their old word is still right
 * somewhere else, so only the glossary records them.
 */
export const WORDS = [
  { say: 'run record', dont: 'receipt', means: 'what a run changed and did', ban: /\breceipts?\b/i },
  { say: 'output / error output', dont: 'stdout, stderr', means: 'what a command printed', ban: /\bstd(?:out|err)\b/i },
  { say: 'the conversation', dont: 'session id, session key', means: 'the thread the agent works in', ban: /\bsession (?:id|key)\b/i },
  { say: 'request', dont: 'IPC', means: 'what the window asks Hollow to do', ban: /\bIPC\b/ },
  { say: 'request to Hollow', dont: 'backend call', means: 'the same, from an agent or a view', ban: /\bbackend call\b/i },
  { say: 'automation', dont: 'workflow', means: 'a saved prompt Hollow runs on a schedule or a trigger', ban: /\bworkflows?\b/i },
  { say: 'Import', dont: 'ingest, Quick ingest', means: 'bring a file or a link into the Library', ban: /\bingest(?:s|ed|ing)?\b/i },
  { say: 'Tonight (the night window)', dont: 'night shift, overnight shift', means: 'the hours Hollow runs queued work unattended, within the overnight cap', ban: /\b(?:night|overnight) shift\b/i },
  { say: 'History & recovery', dont: 'Vault History', means: 'every change Hollow and its agents made, and how to undo it', ban: /\bVault History\b/i },
  { say: 'Draft for the COO', dont: 'Do it (Today, Weekly review)', means: 'puts the suggestion in the COO’s message box; you send it' },
  { say: 'Apply ticked', dont: 'Do it (Tidy up)', means: 'applies the ticked lines of a tidy-up and nothing else' },
  { say: 'Do it', dont: 'Do it on a button that only drafts or applies', means: 'your answer to a COO suggestion (Do it / Not now / Never); Ask’s scopes say what their button does (“Ask the COO”, “Start the chat”; P1-11)' },
  { say: 'Running', dont: 'Activity (Work’s view)', means: 'every running and queued agent turn, and what just finished' },
  { say: 'History', dont: 'Activity (a workspace’s section)', means: 'a workspace’s runs, changes and conversations over time (History & recovery and Conversation history are their own places)' },
  { say: 'Background tasks', dont: 'Activity, Background activity (the dock)', means: 'long operations in the corner dock' },
  { say: 'Recent notifications', dont: 'Recent activity (Review)', means: 'the notification history under Review' },
  { say: 'Last active', dont: 'Recent activity (Work’s sort)', means: 'workspaces in the order you last used them' },
  { say: 'Log', dont: 'Activity (a task’s detail)', means: 'the dated entries in a task note' },
  { say: 'Summary', dont: 'Visual (a section’s toolbar)', means: 'a section’s own layout, as opposed to its canvas' },
  { say: 'Blocked', dont: 'Waiting (Tasks)', means: 'a task held by its status or an open blocker' },
  { say: 'Library', dont: 'vault (for where notes live)', means: 'the notes, imports and results Hollow shows; the Hollow folder is where they sit on disk' },
  { say: 'Silent heartbeat', dont: 'Heartbeat, background check-in', means: 'the quiet agent pass over what changed in your notes, calendar and tasks' },
  { say: 'the chat’s agent’s own mode names (Accept edits, Ask for approval, Skip permissions)', dont: 'Edits only, Full access in a chat’s menus', means: 'a chat names its permission mode as its agent does (per-agent menus, 2026-11-12); Edits only and Bypass all stay for automations, which pick no agent’s words' },
  { say: 'Waiting on you', dont: 'Waiting for a blocked task', means: 'Review: decisions and interventions that need you (a plain “waiting for…” loading line is fine)' },
];

/** Word → what the app says instead (the `ban` rows of WORDS). */
export const PLAIN_WORDS = WORDS.filter(w => w.ban).map(w => [w.ban, `${w.say} (${w.means})`]);

const GLOSSARY = 'docs/reference/glossary.md';
const TABLE_START = '<!-- words:start (scripts/check-plain-words.mjs --glossary writes this table) -->';
const TABLE_END = '<!-- words:end -->';

/** The glossary's "Words we use" table, from WORDS. */
export function glossaryTable() {
  return ['| Say | Don’t say | Means |', '|---|---|---|', ...WORDS.map(w => `| ${w.say} | ${w.dont} | ${w.means} |`)].join('\n');
}

/** The table as it stands in the glossary, or null when the markers are missing. */
export function glossaryTableIn(text) {
  const a = text.indexOf(TABLE_START), b = text.indexOf(TABLE_END);
  return a < 0 || b < a ? null : text.slice(a + TABLE_START.length, b).trim();
}

/** [file or folder (ending in /), text it contains, why it may stay]. */
export const ALLOWED = [
  ['src/appBridge.ts', 'session key', 'an error for the agent calling the canvas bridge, not shown to the person'],
  ['src/store/', 'no session id yet', 'page context sent to the agent (runtime.ts, or a controller split out of it)'],
];

const SKIP_FILE = /\.(test|spec)\.tsx?$|\.d\.ts$|(^|\/)mock[^/]*\.tsx?$|SmokeHarness|inventoryMap\.ts$|routeLedger\.ts$/;
const SKIP_DIR = /node_modules|\/canvas\/kit(\/|$)|\/generated(\/|$)/;
const NOT_TEXT_ATTR = /^(class|classList|id|for|href|key|name|role|type|data-[\w-]+|aria-(controls|labelledby|describedby|owns))$/;
const NOT_TEXT_CALL = /\b(invoke|api\.\w+|listen|emit|querySelector(All)?|closest|matches|startsWith|endsWith|includes|split|replace|test|getItem|setItem|addEventListener|removeEventListener|console\.\w+|require|import|RegExp)$/;

function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) { if (!SKIP_DIR.test(p)) walk(p, out); }
    else if (/\.tsx?$/.test(e.name)) out.push(p);
  }
  return out;
}

function literalText(n) {
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
  if (ts.isTemplateExpression(n)) return n.head.text + n.templateSpans.map(s => ` … ${s.literal.text}`).join('');
  return null;
}

/** Whether a literal sits where its text is never shown. */
function notShown(n) {
  for (let p = n.parent, child = n; p; child = p, p = p.parent) {
    if (ts.isJsxAttribute(p)) return NOT_TEXT_ATTR.test(p.name.getText());
    if (ts.isImportDeclaration(p) || ts.isExportDeclaration(p) || ts.isLiteralTypeNode(p) || ts.isCaseClause(p)) return true;
    if (ts.isElementAccessExpression(p) && p.argumentExpression === child) return true;
    if (ts.isBinaryExpression(p) && /^[!=]==?$/.test(p.operatorToken.getText())) return true;
    if (ts.isCallExpression(p) && p.arguments.includes(child)) return NOT_TEXT_CALL.test(p.expression.getText());
    if (ts.isStatement(p) || ts.isJsxElement(p) || ts.isJsxSelfClosingElement(p)) return false;
  }
  return false;
}

/** Whether a literal is JSX content: `{'…'}`, or a branch of `?:`, `&&`, `||`
 *  or `??` inside one. */
function jsxContent(n) {
  let child = n;
  for (let p = n.parent; p; child = p, p = p.parent) {
    if (ts.isParenthesizedExpression(p)) continue;
    if (ts.isConditionalExpression(p) && p.condition !== child) continue;
    if (ts.isBinaryExpression(p) && /^(&&|\|\||\?\?)$/.test(p.operatorToken.getText()) && p.right === child) continue;
    if (!ts.isJsxExpression(p) || !p.parent) return false;
    if (ts.isJsxFragment(p.parent)) return true;
    return ts.isJsxElement(p.parent) && !/^(style|script)$/.test(p.parent.openingElement.tagName.getText());
  }
  return false;
}

/** Whether a literal is the fallback of `||` or `??` (or `||=` / `??=`, P1-03): shown in place
 *  of a missing value, as in `${error || 'the build has no comparison
 *  record'}`, so a lower-case one is read too. */
function fallback(n) {
  let child = n;
  let p = n.parent;
  while (p && ts.isParenthesizedExpression(p)) { child = p; p = p.parent; }
  return !!p && ts.isBinaryExpression(p) && /^(\|\||\?\?)=?$/.test(p.operatorToken.getText()) && p.right === child;
}

/** The literal texts a toast can start with: the message itself, either
 *  branch of `?:`, or the fallback of `||` / `??` (P1-04). A template that
 *  starts with a value isn't judged. */
function toastStarts(n) {
  while (ts.isParenthesizedExpression(n)) n = n.expression;
  if (ts.isConditionalExpression(n)) return [...toastStarts(n.whenTrue), ...toastStarts(n.whenFalse)];
  if (ts.isBinaryExpression(n) && /^(\|\||\?\?)$/.test(n.operatorToken.getText())) return toastStarts(n.right);
  if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return [[n, n.text]];
  if (ts.isTemplateExpression(n)) return n.head.text ? [[n, n.head.text]] : [];
  return [];
}

/** Problems in one file's text: `file:line: word in "sentence" (say: …)`, and
 *  a toast (`flashFooter`) that starts with "+ " or a lower-case letter. */
export function checkSource(file, text) {
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, file.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const out = [];
  const visit = n => {
    if (ts.isCallExpression(n) && /(^|\.)flashFooter$/.test(n.expression.getText()) && n.arguments[0]) {
      for (const [lit, t] of toastStarts(n.arguments[0])) {
        if (!/^(\+ |[a-z])/.test(t)) continue;
        const line = sf.getLineAndCharacterOfPosition(lit.getStart()).line + 1;
        out.push(`${file}:${line}: toast "${t.replace(/\s+/g, ' ').trim().slice(0, 60)}" starts with ${t.startsWith('+ ') ? '"+ "' : 'a lower-case letter'} (say it as a sentence: "Copied", "Automation added")`);
      }
    }
    const jsx = ts.isJsxText(n);
    const t = jsx ? n.text : literalText(n);
    if (t !== null && !notShown(n)) {
      const sentence = /[A-Za-z]{2,}\s+[A-Za-z’']{2,}/.test(t) && (jsx || jsxContent(n) || fallback(n) || /^[\s(“"]*[A-Z0-9…·—]/.test(t));
      if (sentence) {
        for (const [word, instead] of PLAIN_WORDS) {
          const m = t.match(word);
          if (!m) continue;
          if (ALLOWED.some(([f, s]) => (f.endsWith('/') ? file.startsWith(f) : file === f) && t.includes(s))) continue;
          const line = sf.getLineAndCharacterOfPosition(n.getStart()).line + 1;
          out.push(`${file}:${line}: "${m[0]}" in "${t.replace(/\s+/g, ' ').trim().slice(0, 90)}" (say: ${instead})`);
        }
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
  return out;
}

/** Current route names in release instructions after 2026-10-13. Historical
 *  notes stay historical; the explicit old-to-new glossary paragraph is exempt. */
export function checkReleaseNames(md) {
  const problems = [];
  for (const section of md.split(/(?=^# )/m)) {
    const title = section.split('\n')[0];
    const date = title.match(/(20\d{2}-\d{2}-)(\d{2})(?:\/(\d{2}))?/);
    if (!date || `${date[1]}${date[3] ?? date[2]}` <= '2026-10-13') continue;
    for (const paragraph of section.split(/\n\s*\n/)) {
      if (paragraph.includes('words that named several things now name one')) continue;
      const retired = /\bMission Control\b|(?:Your work|Work)\s*→\s*(?:\*\*)?(?:Activity|Overview)\b|workspace(?:’s|'s| panel)?\s*→\s*Activity\b|→\s*Visual\b|\bRecent activity\b/;
      if (retired.test(paragraph)) problems.push(`docs/WHATS_NEW.md: ${title}: ${paragraph.trim().slice(0, 140)}`);
    }
  }
  return problems;
}

export function checkRepo(dir = root) {
  const problems = [];
  for (const f of walk(join(dir, 'src'))) {
    const rel = relative(dir, f).split('\\').join('/');
    if (SKIP_FILE.test(rel)) continue;
    problems.push(...checkSource(rel, readFileSync(f, 'utf8')));
  }
  problems.push(...checkReleaseNames(readFileSync(join(dir, 'docs/WHATS_NEW.md'), 'utf8')));
  return problems;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1] && process.argv[2] === '--glossary') {
  const path = join(root, GLOSSARY);
  const text = readFileSync(path, 'utf8');
  if (glossaryTableIn(text) === null) { console.error(`${GLOSSARY}: the words:start / words:end markers are missing`); process.exit(1); }
  const a = text.indexOf(TABLE_START) + TABLE_START.length, b = text.indexOf(TABLE_END);
  writeFileSync(path, `${text.slice(0, a)}\n${glossaryTable()}\n${text.slice(b)}`);
  console.log(`check-plain-words: wrote the Words we use table in ${GLOSSARY}`);
} else if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const problems = checkRepo();
  if (problems.length) {
    console.error(`${problems.length} engineering word(s) in what Hollow says:`);
    for (const p of problems) console.error(`  ${p}`);
    process.exit(1);
  }
  console.log('check-plain-words: no engineering words in shown text');
}
