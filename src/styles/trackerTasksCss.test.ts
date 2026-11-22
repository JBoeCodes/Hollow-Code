import { describe, expect, it } from 'vitest';
import css from './tracker-tasks.css?raw';

/** The body of the `@container task-detail (max-width: 45em)` block. */
function narrowBlock(): string {
  const at = css.indexOf('@container task-detail (max-width: 45em) {');
  expect(at).toBeGreaterThan(-1);
  let depth = 0;
  for (let i = css.indexOf('{', at); i < css.length; i += 1) {
    if (css[i] === '{') depth += 1;
    if (css[i] === '}') {
      depth -= 1;
      if (depth === 0) return css.slice(at, i);
    }
  }
  throw new Error('unterminated container block');
}

describe('issue detail narrow layout', () => {
  it('puts the properties first in the side pane inside the task-detail container query', () => {
    expect(narrowBlock()).toMatch(/\.issue-detail\[data-mode="pane"\] \.issue-properties \{[^}]*order: -1/);
  });

  it('keeps the full page rule and leaves the wide layout alone', () => {
    expect(narrowBlock()).toMatch(/\.issue-detail\[data-mode="page"\] \.issue-properties \{[^}]*order: -1/);
    expect(css).not.toMatch(/^\.issue-detail\[data-mode="pane"\] \.issue-properties/m);
  });
});
