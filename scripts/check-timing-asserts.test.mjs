import assert from 'node:assert/strict';
import { test } from 'node:test';
import { check, loadAllowlist, repoFiles, scanSource } from './check-timing-asserts.mjs';

const texts = offenders => offenders.map(o => o.text);

test('the repository passes: no new tight bound, and no stale allowlist entry', () => {
  const allowlist = loadAllowlist();
  assert.deepEqual(check(repoFiles(), allowlist), { newOffenders: [], stale: [] });
  // The list only shrinks: removing an entry fails here until the pin comes
  // down with it. Raising it needs a reason in the PR (a rule that now sees an
  // old bound, say), never a new tight bound: give that one `// timing: <reason>`.
  assert.equal(allowlist.length, 3, `${allowlist.length} allowlisted, the pin is 3`);
});

test('a small upper bound on elapsed() inside an assert is flagged, with its line', () => {
  const src = ['fn t() {', '    assert!(t.elapsed() < Duration::from_millis(500));', '}'].join('\n');
  assert.deepEqual(scanSource('a.rs', src), [
    { path: 'a.rs', line: 2, text: 'assert!(t.elapsed() < Duration::from_millis(500));' },
  ]);
});

test('`// timing: <reason>` on the line, or on the line above, accepts it', () => {
  assert.deepEqual(
    scanSource('a.rs', 'assert!(t.elapsed() < Duration::from_millis(500)); // timing: pure CPU, 100x margin'),
    [],
  );
  assert.deepEqual(
    scanSource('a.rs', '// timing: pure CPU, 100x margin\nassert!(t.elapsed() < Duration::from_millis(500));'),
    [],
  );
  assert.equal(scanSource('a.rs', '// timing:\nassert!(t.elapsed() < Duration::from_millis(500));').length, 1, 'a reason is required');
  assert.equal(
    scanSource('a.rs', '// timing: pure CPU\nlet x = 1;\nassert!(t.elapsed() < Duration::from_millis(500));').length,
    1,
    'only the line directly above counts',
  );
});

test('wide bounds, lower bounds, loops and comments are not flagged', () => {
  for (const line of [
    'assert!(t.elapsed() < Duration::from_secs(30));',
    'assert!(t.elapsed() < Duration::from_secs(5));',
    'assert!(t.elapsed() < Duration::from_millis(2000));',
    'assert!(t.elapsed() >= Duration::from_millis(400));',
    'assert!(Duration::from_millis(400) <= t.elapsed());',
    'while began.elapsed() < Duration::from_secs(1) {',
    '// assert!(t.elapsed() < Duration::from_millis(500));',
    'if at.elapsed() < Duration::from_millis(500) { return; }',
    'assert!(t.elapsed() < deadline);',
  ]) {
    assert.deepEqual(scanSource('a.rs', line), [], line);
  }
});

test('the other spellings: <=, as_millis, the reversed form, a bound variable and a multi-line assert', () => {
  const src = [
    'assert!(t.elapsed() <= std::time::Duration::from_secs(2));',
    'assert!(t.elapsed().as_millis() < 700);',
    'assert!(Duration::from_millis(1_500) > t.elapsed());',
    'let took = started.elapsed();',
    'assert!(took < Duration::from_millis(550), "{took:?}");',
    'assert!(',
    '    started.elapsed() < Duration::from_secs(4),',
    '    "{:?}",',
    ');',
    'assert!(',
    '    started.elapsed()',
    '        < Duration::from_secs(3),',
    ');',
  ].join('\n');
  assert.deepEqual(
    scanSource('a.rs', src).map(o => o.line),
    [1, 2, 3, 5, 7, 12],
  );
});

test('a bare number bounding a .ms or _ms field, or a name bound from elapsed()', () => {
  const src = [
    'assert!(mark.unwrap().ms < 700);',
    'assert!(map.took_ms <= 1_500, "{} ms", map.took_ms);',
    'let ms = t.elapsed().as_millis();',
    'assert!(ms < 500);',
    'let secs = t.elapsed().as_secs_f64();',
    'assert!(secs < 1.5, "{secs}");',
  ].join('\n');
  assert.deepEqual(scanSource('a.rs', src).map(o => o.line), [1, 2, 4, 6]);
  for (const text of [
    'assert!(mark.ms < 2_000);',
    'assert!(map.took_ms < 2000);',
    'assert!(ma.ms >= 300 && mb.ms >= 300);',
    'let secs = t.elapsed().as_secs();\nassert!(secs < 30);',
    'let ms = t.elapsed().as_millis();\nif ms < 500 { return; }',
    'assert!(count < 5);',
    'assert!(other.ms_total < 5);',
  ]) {
    assert.deepEqual(scanSource('a.rs', text), [], text);
  }
});

test('the end of a range holding an elapsed time', () => {
  const src = [
    'assert!((150..1200).contains(&mark.ms), "{mark:?}");',
    'assert!((0..=900).contains(&t.elapsed().as_millis()));',
    'assert!((0.5..2.5).contains(&t.elapsed().as_secs_f64()));',
    'assert!((Duration::from_millis(100)..Duration::from_millis(900)).contains(&t.elapsed()));',
    'let took = t.elapsed().as_millis();',
    'assert!((10..1000).contains(&took));',
  ].join('\n');
  assert.deepEqual(scanSource('a.rs', src).map(o => o.line), [1, 2, 3, 4, 6]);
  for (const text of [
    'assert!((150..2000).contains(&mark.ms));',
    'assert!((0.5..30.0).contains(&t.elapsed().as_secs_f64()));',
    'assert!((Duration::from_millis(100)..Duration::from_secs(30)).contains(&t.elapsed()));',
    'assert!((1500..1900).contains(&port));',
    'assert!((1..=6).contains(&hashes));',
    'if (150..1200).contains(&mark.ms) { return; }',
  ]) {
    assert.deepEqual(scanSource('a.rs', text), [], text);
  }
});

test('as_secs_f64() and as_secs_f32() under 5', () => {
  const src = [
    'assert!(t.elapsed().as_secs_f64() < 1.5);',
    'assert!(t.elapsed().as_secs_f32() <= 4.9);',
    'assert!(t.elapsed().as_secs_f64() < 3, "{:?}", t);',
  ].join('\n');
  assert.deepEqual(scanSource('a.rs', src).map(o => o.line), [1, 2, 3]);
  for (const text of [
    'assert!(t.elapsed().as_secs_f64() < 5.0);',
    'assert!(t.elapsed().as_secs_f64() < 30.0);',
    'assert!(t.elapsed().as_secs_f64() >= 0.3);',
  ]) {
    assert.deepEqual(scanSource('a.rs', text), [], text);
  }
});

test('a bound outside any assert is not flagged, even right after one', () => {
  const src = ['assert!(ok);', 'let fast = t.elapsed() < Duration::from_millis(500);'].join('\n');
  assert.deepEqual(scanSource('a.rs', src), []);
});

test('check: an allowlisted line passes, a new one fails, an entry with no line is stale', () => {
  const files = [
    { path: 'src-tauri/src/a.rs', text: 'assert!(t.elapsed() < Duration::from_millis(500));\nassert!(u.elapsed() < Duration::from_secs(1));' },
  ];
  const allowlist = [
    { path: 'src-tauri/src/a.rs', text: 'assert!(t.elapsed() < Duration::from_millis(500));' },
    { path: 'src-tauri/src/a.rs', text: 'assert!(gone.elapsed() < Duration::from_millis(500));' },
  ];
  const { newOffenders, stale } = check(files, allowlist);
  assert.deepEqual(texts(newOffenders), ['assert!(u.elapsed() < Duration::from_secs(1));']);
  assert.deepEqual(stale, [{ path: 'src-tauri/src/a.rs', text: 'assert!(gone.elapsed() < Duration::from_millis(500));' }]);
});

test('check: an entry is keyed by path and text, and covers one line', () => {
  const line = 'assert!(t.elapsed() < Duration::from_millis(500));';
  const twice = [{ path: 'b.rs', text: `${line}\n${line}` }];
  const one = [{ path: 'b.rs', text: line }];
  assert.equal(check(twice, one).newOffenders.length, 1, 'the second copy is new');
  assert.equal(check([{ path: 'c.rs', text: line }], one).newOffenders.length, 1, 'another file is new');
  assert.deepEqual(check([{ path: 'c.rs', text: line }], one).stale, one);
  assert.deepEqual(check([{ path: 'b.rs', text: `\n\n    ${line}` }], one), { newOffenders: [], stale: [] }, 'not line numbers');
});
