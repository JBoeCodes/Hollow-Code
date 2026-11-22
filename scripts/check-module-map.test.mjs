import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkMap, checkModuleMaps, expand, section, tokenRegex, tokens } from './check-module-map.mjs';

test('the repository passes: every module has a line and every line a module', () => {
  assert.deepEqual(checkModuleMaps(), []);
});

test('a module with no line, and a line with no module, are both problems', () => {
  const problems = checkMap({
    name: 'MAP',
    files: ['a.rs', 'b.rs', 'self_install_x_tests.rs'],
    mapTokens: ['a.rs', 'gone.rs', 'self_install_*_tests.rs', 'process_group/{orphans,reaper}.rs'],
    all: new Set(['a.rs', 'b.rs', 'self_install_x_tests.rs', 'process_group/orphans.rs']),
  });
  assert.deepEqual(problems, [
    'b.rs has no line in MAP',
    "MAP names gone.rs, which doesn't exist",
    "MAP names process_group/reaper.rs, which doesn't exist",
  ]);
});

test('a brace list stands for each name, and a glob must match something', () => {
  assert.deepEqual(expand('process_group/{orphans,reaper}.rs'), ['process_group/orphans.rs', 'process_group/reaper.rs']);
  const problems = checkMap({
    name: 'MAP',
    files: [],
    mapTokens: ['commands/*_api.rs', 'src-tauri/src/*.rs'],
    all: new Set(['commands.rs']),
  });
  assert.deepEqual(problems, ["MAP names commands/*_api.rs, which doesn't exist"], 'a prose path is not an entry');
});

test('globs and brace lists match as written, nothing more', () => {
  assert.ok(tokenRegex('self_install_*_tests.rs').test('self_install_receipt_tests.rs'));
  assert.ok(!tokenRegex('self_install_*_tests.rs').test('self_install.rs'));
  assert.ok(tokenRegex('commands/{a,b}_api.rs').test('commands/b_api.rs'));
  assert.ok(!tokenRegex('a.rs').test('xa.rs'), 'a dot is a dot, not any character');
});

test('only the named section is read, and only backticked tokens with the extension', () => {
  const doc = '# T\n### Backend module map\n| x | `a.rs` (`b.rs`), `c.ts` |\n### AppState\n`d.rs`';
  assert.deepEqual(tokens(section(doc, '### Backend module map'), '.rs'), ['a.rs', 'b.rs']);
});
