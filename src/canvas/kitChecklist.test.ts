import { describe, expect, it } from 'vitest';
import { checklistRows } from './kit/checklist';
import { escapeRowTags } from './strayTags';

describe('canvas checklist rows', () => {
  it('reads checked, unchecked and plain rows without guessing completion', () => {
    expect(checklistRows(['- [x] Build', '[X] Review', '- [ ] Install', 'By hand', '  ', '- Plain'])).toEqual({
      rows: [{ label: 'Build', done: true }, { label: 'Review', done: true }, { label: 'Install', done: false }, { label: 'By hand', done: false }, { label: 'Plain', done: false }], repeated: 0,
    });
  });
  it('keeps literal labels, first repeated row and discloses repeats', () => {
    expect(checklistRows(['[ ] __proto__', '[x] constructor', '[ ] constructor', '[x]', '[ ] <title>'])).toEqual({
      rows: [{ label: '__proto__', done: false }, { label: 'constructor', done: true }, { label: '<title>', done: false }], repeated: 1,
    });
  });
  it('rescues stray tags in checklist text before parsing', () => {
    expect(escapeRowTags('<hc-checklist>[ ] Back to <title></hc-checklist>')).toBe('<hc-checklist>[ ] Back to &lt;title></hc-checklist>');
  });
});
