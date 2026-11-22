// Every `#[ignore]`d Rust test says why (overnight 2026-10-16, B09). The
// reason shows in `cargo test -- --ignored --list` and in every run's
// "ignored, <reason>" line, and a bare `#[ignore]` is how a failing test gets
// quietly switched off. Write `#[ignore = "why; how to run it"]`, and the same
// inside a `#[cfg_attr(<cfg>, ignore = "…")]`. A reason may start on the next
// line (`#[ignore =` at the end of one line, the string on the next). Lines
// whose trimmed text starts with `//` (comments and doc comments that mention
// the attribute) are skipped. A text check over src-tauri/src, every folder deep.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src-tauri', 'src');

// The `ignore` of `#[ignore …` or of `#[cfg_attr(<cfg>, …, ignore …)]`. Each
// one needs `= "…"` after it (a raw string `= r"…"` / `= r#"…"#` counts as a
// reason too).
const IGNORE = /#\[\s*(?:cfg_attr\s*\([^\]]*?,\s*)?ignore\b/g;
const REASON = /^\s*=\s*r?#*"/;
// `=` ends the line: the reason is the string that starts the next one.
const REASON_BELOW = /^\s*=\s*$/;
const STRING_START = /^\s*r?#*"/;

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith('.rs') ? [path] : [];
  });
}

/** The 1-based line numbers of `text` that hold a bare `#[ignore]`. */
export function bareIgnores(text) {
  const all = text.split('\n');
  const lines = [];
  all.forEach((line, i) => {
    if (line.trim().startsWith('//')) return;
    const bare = [...line.matchAll(IGNORE)].some(match => {
      const rest = line.slice(match.index + match[0].length);
      if (REASON.test(rest)) return false;
      return !(REASON_BELOW.test(rest) && STRING_START.test(all[i + 1] ?? ''));
    });
    if (bare) lines.push(i + 1);
  });
  return lines;
}

test('the pattern catches a bare ignore and passes one with a reason', () => {
  assert.deepEqual(bareIgnores('#[test]\n#[ignore]\nfn a() {}'), [2]);
  assert.deepEqual(bareIgnores('    #[ignore ]\n    #[ignore]'), [1, 2]);
  assert.deepEqual(bareIgnores('#[test] #[ignore] fn b() {}'), [1]);
  assert.deepEqual(bareIgnores('#[ignore = "bench: run with --ignored"]'), []);
  assert.deepEqual(bareIgnores('#[ignore="needs the network"]'), []);
  assert.deepEqual(bareIgnores('#[ignore = r"raw reason"]'), []);
  assert.deepEqual(bareIgnores('//! `report` is `#[ignore]`d\n    // ── benches (#[ignore]) ──'), []);
  assert.deepEqual(bareIgnores('/// a doc line: #[ignore]\n#[ignored_attr]'), []);
});

test('a cfg_attr ignore needs a reason too', () => {
  assert.deepEqual(bareIgnores('#[test]\n#[cfg_attr(miri, ignore)]\nfn a() {}'), [2]);
  assert.deepEqual(bareIgnores('#[cfg_attr(not(target_os = "macos"), ignore)]'), [1]);
  assert.deepEqual(bareIgnores('#[cfg_attr(windows, ignore, allow(dead_code))]'), [1]);
  assert.deepEqual(bareIgnores('fn a() {}\n\n    #[cfg_attr( miri , ignore )]'), [3]);
  assert.deepEqual(bareIgnores('#[cfg_attr(miri, ignore = "miri can\'t open sqlite")]'), []);
  assert.deepEqual(bareIgnores('#[cfg_attr(miri, ignore = r#"raw "reason""#)]'), []);
  assert.deepEqual(bareIgnores('#[cfg_attr(feature = "ignore", allow(dead_code))]'), []);
  assert.deepEqual(bareIgnores('#[cfg_attr(test, ignored_attr)]'), []);
  assert.deepEqual(bareIgnores('// #[cfg_attr(miri, ignore)]'), []);
});

test('a reason that starts on the next line counts', () => {
  assert.deepEqual(bareIgnores('#[test]\n#[ignore =\n    "a long reason"]\nfn a() {}'), []);
  assert.deepEqual(bareIgnores('#[ignore =\n    r"raw reason"\n]'), []);
  assert.deepEqual(bareIgnores('#[cfg_attr(miri, ignore =\n    "split reason")]'), []);
  assert.deepEqual(bareIgnores('#[ignore =\n]\nfn a() {}'), [1]);
  assert.deepEqual(bareIgnores('fn a() {}\n#[ignore ='), [2]);
  assert.deepEqual(bareIgnores('#[ignore]\n"not a reason"'), [1]);
});

test('the walk reaches nested modules', () => {
  const files = rustFiles(src).map(file => relative(src, file));
  assert.ok(files.includes('lib.rs'), 'lib.rs is listed');
  assert.ok(files.some(file => file.includes('/')), 'a file in a sub-folder is listed');
});

test('every #[ignore] in src-tauri/src carries a reason', () => {
  const found = [];
  for (const file of rustFiles(src)) {
    const name = relative(root, file);
    for (const line of bareIgnores(readFileSync(file, 'utf8'))) found.push(`${name}:${line}`);
  }
  const fix = 'write #[ignore = "why; how to run it"] (or cfg_attr(…, ignore = "…"))';
  assert.deepEqual(found, [], `${fix} at:\n${found.join('\n')}`);
});
