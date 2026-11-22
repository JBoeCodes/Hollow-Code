import assert from 'node:assert/strict';
import { test } from 'node:test';
import { checkRepo, dynamicPrefixes, findOrphans, isBuilt, selectorClasses, sourceTokens } from './check-orphan-css.mjs';

test('the repository passes: every styled class is used', () => {
  assert.deepEqual(checkRepo(), []);
});

test('classes come from selectors, not declarations, comments, strings or attribute values', () => {
  const css = `
    /* .in-comment {} */
    .a, .b > .c:hover { content: ".in-string"; background: url(x.png); }
    @media (max-width: 900px) { .d .e { width: 1.5px; } }
    .f[data-x=".in-attr"] { }
  `;
  assert.deepEqual([...selectorClasses(css)].sort(), ['a', 'b', 'c', 'd', 'e', 'f']);
});

test('a class is used when a source names it exactly, or builds it from a prefix', () => {
  const tokens = sourceTokens('<div class="kept work-row">{`kind-${k}`}</div> h.l1');
  const prefixes = dynamicPrefixes("class={`kind-${k}`} 'diff-' + t `l${level}`");
  assert.deepEqual([...prefixes].sort(), ['diff-', 'kind-', 'l']);
  assert.ok(isBuilt('kind-error', prefixes));
  assert.ok(isBuilt('l3', prefixes), 'digits after a dashless prefix');
  assert.ok(!isBuilt('library', prefixes), 'letters after a dashless prefix are not built from it');
  const orphans = findOrphans({
    css: new Map([['a.css', '.kept{} .work-row{} .kind-error{} .l3{} .work-tabs{} .library{}']]),
    tokens,
    prefixes,
    library: new Set(),
  });
  assert.deepEqual(orphans.map(o => o.cls), ['library', 'work-tabs']);
});

test('an id, link or key built from a prefix is not a class builder', () => {
  const prefixes = dynamicPrefixes('<section id={`settings-${id}`} aria-labelledby={`settings-${id}-h`} class={`row-${kind}`}> data-x="tab-" + n');
  assert.deepEqual([...prefixes].sort(), ['row-']);
  const orphans = findOrphans({ css: new Map([['a.css', '.settings-role-row{} .row-open{}']]), tokens: new Set(), prefixes, library: new Set() });
  assert.deepEqual(orphans.map(o => o.cls), ['settings-role-row']);
});

test('in the CodeMirror sheet only CodeMirror\'s own cm-* classes are skipped', () => {
  const css = new Map([['src/styles/codemirror.css', '.cm-scroller{} .cm-tooltip-autocomplete{} .cm-md-gone{} .vault-editor-gone{}'], ['src/styles/a.css', '.cm-elsewhere{}']]);
  const orphans = findOrphans({ css, tokens: new Set(), prefixes: new Set(), library: new Set() });
  assert.deepEqual(orphans.map(o => o.cls), ['cm-elsewhere', 'cm-md-gone', 'vault-editor-gone'], 'the app\'s own cm-md-* decorations are checked');
});

test('a near name is not a reference: work-tabs is not kept by background-work-tabs', () => {
  const tokens = sourceTokens('class="background-work-tabs"');
  const prefixes = dynamicPrefixes('id={`background-work-${id}`}');
  const orphans = findOrphans({ css: new Map([['a.css', '.work-tabs{}']]), tokens, prefixes, library: new Set() });
  assert.deepEqual(orphans.map(o => o.cls), ['work-tabs']);
});
