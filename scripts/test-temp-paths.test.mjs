// Rust tests must not share a fixed folder under `$TMPDIR`. Two worktrees
// running `cargo test` with one TMPDIR (the 2026-10-09/10 overnight lanes)
// deleted each other's `hollow-operator-<tag>` fixtures mid-test. A folder
// named only by a literal, or by `format!` with nothing unique in it, is
// refused; use `tempfile`, or add `std::process::id()` or a random part as a
// `format!` argument. A text check, so it can't follow a temp dir bound to a
// variable first, a constant name, or a positional `{}` filled with something
// that isn't unique; review those by hand.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = join(root, 'src-tauri', 'src');

// Shared on purpose, each with its reason.
const ALLOWED = new Map([
  // Runtime fallbacks when Application Support is missing, not test folders.
  ['lease_lock.rs', ['"hollow-leases"']],
  ['run_permit.rs', ['"hollow-run-permits"']],
  // An ignored benchmark's output folder, kept stable so runs can be compared.
  ['prep_bench.rs', ['"hollow-prep-bench"']],
  // A shared base: each entry is owner-marked with its pid, a live owner's name
  // falls back to a one-off unique folder, and only dead owners' are swept.
  ['scratch.rs', ['"hollow-run-tests"']],
  // The peer tunnels' socket folder: 0700, checked to be this user's, a real folder (not a
  // symlink) before any socket goes in it, and sockets named by machine id.
  ['peer_tunnel.rs', ['"hollow-peer"']],
]);

// `temp_dir().join("x")` or `temp_dir().join(&format!("x-{tag}"))` with no
// further format arguments (so nothing per-process or random), raw strings too.
const FIXED = /temp_dir\(\)\s*\.join\(\s*&?\s*(?:(r?"[^"]*")\s*\)|format!\(\s*(r?"[^"]*")\s*,?\s*\)\s*\))/g;

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith('.rs') ? [path] : [];
  });
}

function fixedTempPaths(text) {
  return [...text.matchAll(FIXED)].map(m => m[1] ?? m[2]);
}

test('the pattern catches the old fixtures and passes unique ones', () => {
  assert.deepEqual(fixedTempPaths('std::env::temp_dir().join(format!("hollow-operator-{tag}"))'), [
    '"hollow-operator-{tag}"',
  ]);
  assert.deepEqual(fixedTempPaths('let d = std::env::temp_dir().join("hollow-people-list");'), [
    '"hollow-people-list"',
  ]);
  assert.deepEqual(fixedTempPaths('temp_dir().join(&format!("hollow-a-{tag}",))'), ['"hollow-a-{tag}"']);
  assert.deepEqual(fixedTempPaths('temp_dir().join(r"hollow-b")'), ['r"hollow-b"']);
  assert.deepEqual(
    fixedTempPaths('std::env::temp_dir().join(format!("hollow-x-{}", std::process::id()))'),
    [],
  );
  assert.deepEqual(
    fixedTempPaths('std::env::temp_dir().join(format!(\n    "hollow-y-{}-{}",\n    std::process::id(),\n    rand::random::<u32>()\n))'),
    [],
  );
});

test('no Rust source names a fixed folder under the temp dir', () => {
  const found = [];
  for (const file of rustFiles(src)) {
    const name = relative(src, file);
    const allowed = ALLOWED.get(name) ?? [];
    for (const literal of fixedTempPaths(readFileSync(file, 'utf8'))) {
      if (!allowed.includes(literal)) found.push(`${name}: ${literal}`);
    }
  }
  assert.deepEqual(found, [], 'use tempfile, or pass std::process::id() or a random part as a format! argument');
});
