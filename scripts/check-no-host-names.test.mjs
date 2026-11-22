// Shipped-code hygiene check: Hollow ships no personal host names, tailnet
// domains, private LAN IPs, or personal user home directories in bundled app
// code (src/, src-tauri/src/, and app config files). Tests, mocks (kept out of
// the bundle by check-no-mock-bundle), and Rust #[cfg(test)] modules are
// excluded.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = join(dirname(fileURLToPath(import.meta.url)), '..');

export const SHIPPED = [
  'src',
  'src-tauri/src',
  'src-tauri/tauri.conf.json',
  'src-tauri/tauri.candidate.conf.json',
  'src-tauri/Cargo.toml',
  'src-tauri/Info.plist',
  'src-tauri/Entitlements.plist',
  'src-tauri/capabilities',
  'index.html',
  'public',
];

/** Patterns that must never ship inside production app code. */
export const NAMES = /(?<!example)\.tail[0-9a-f]{4,}\.ts\.net|192\.168\.0\.\d|\/Users\/(?!you\b|me\b|user\b|dev\b|runner\b|builder\b|mock\b|fixture\b|sample\b|someone\b|test\b|Shared\b|USER\b|<)[A-Za-z0-9._-]{3,}\//i;
const NOT_SHIPPED = /\.(test|spec)\.[cm]?[jt]sx?$|_tests\.rs$|(^|\/)tests?\/|__tests__|(^|\/)mock[^/]*\.tsx?$|\.(png|jpe?g|gif|webp|ico|icns|woff2?|ttf|otf|mp4|webm|pdf|zip|gz|bin|onnx)$/i;

/** A Rust file without its `#[cfg(test)] mod ... { ... }` blocks (lines kept
 *  as blanks so line numbers still match). */
export function withoutRustTests(text) {
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) continue;
    const head = lines[i + 1] ?? '';
    if (!/^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{\s*$/.test(head)) continue;
    const indent = head.match(/^\s*/)[0];
    let j = i + 2;
    while (j < lines.length && lines[j] !== `${indent}}`) j++;
    for (let k = i; k <= Math.min(j, lines.length - 1); k++) lines[k] = '';
    i = j;
  }
  return lines.join('\n');
}

/** `path:line: text` for every shipped line matching NAMES. */
export function findNames(files, read) {
  const hits = [];
  for (const file of files) {
    if (NOT_SHIPPED.test(file)) continue;
    let text = read(file);
    if (text == null) continue;
    if (file.endsWith('.rs')) text = withoutRustTests(text);
    text.split('\n').forEach((line, i) => {
      if (NAMES.test(line)) hits.push(`${file}:${i + 1}: ${line.trim().slice(0, 160)}`);
    });
  }
  return hits;
}

test('no personal host, tailnet, or home path ships in the app', () => {
  const files = execFileSync('git', ['-C', repo, 'ls-files', '--', ...SHIPPED], { encoding: 'utf8' }).split('\n').filter(Boolean);
  assert.ok(files.length > 500, `scanned ${files.length} files`);
  const read = file => { try { return readFileSync(join(repo, file), 'utf8'); } catch { return null; } };
  assert.deepEqual(findNames(files, read), []);
});

test('the scan finds personal hosts in shipped code and skips tests, mocks and test modules', () => {
  const files = {
    'src/a.ts': "const host = '192.168.0.42';",
    'src/a.test.ts': "expect(host).toBe('192.168.0.42');",
    'src/mockApi.ts': "host: '192.168.0.42'",
    'src-tauri/src/b.rs': 'fn x() {}\n// /Users/realperson/secret\n#[cfg(test)]\nmod tests {\n    const H: &str = "192.168.0.5";\n}\nconst Y: &str = "box.tail9a8b7c.ts.net";\n',
    'src-tauri/src/b_tests.rs': 'const H: &str = "box.tail9a8b7c.ts.net";',
    'src-tauri/tauri.conf.json': '{ "identifier": "com.hollow.app" }',
    'tests/smoke/x.spec.ts': "'192.168.0.42'",
  };
  const hits = findNames(Object.keys(files), f => files[f]);
  assert.deepEqual(hits.map(h => h.split(': ')[0]), ['src/a.ts:1', 'src-tauri/src/b.rs:2', 'src-tauri/src/b.rs:7']);
});
