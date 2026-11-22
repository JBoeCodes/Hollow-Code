import { describe, expect, it } from 'vitest';
import { copyMeta, keepConfirm } from './conflictCopies';
import { conflictCopiesFixture } from './mockConflictCopies';
import DIALOG from './components/ConflictCopiesDialog.tsx?raw';

describe('conflict copies (overnight A10)', () => {
  const c = { copy: 'notes/Plan 2.md', original: 'notes/Plan.md', identical: false, copyModified: '2026-10-09T10:00:00Z', originalModified: '2026-10-08T09:00:00Z' };

  it('says which file goes to the trash and which stays', () => {
    const orig = keepConfirm(c, false);
    expect(orig.title).toBe('Keep the original?');
    expect(orig.body).toContain('“notes/Plan 2.md” moves to the vault\'s trash');
    expect(orig.confirmLabel).toBe('Keep the original');
    const copy = keepConfirm(c, true);
    expect(copy.title).toBe('Keep the copy?');
    expect(copy.body).toContain('“notes/Plan.md” moves to the vault\'s trash and “notes/Plan 2.md” takes its name');
  });

  it('describes a row by its original, sameness and days', () => {
    expect(copyMeta(c)).toBe('copy of notes/Plan.md · differs · copy 2026-10-09, original 2026-10-08');
    expect(copyMeta({ ...c, identical: true })).toContain('same contents');
  });
});

describe('Not a copy (morning M7)', () => {
  const listed = () => (conflictCopiesFixture('conflict_copies', undefined)!.value as { copy: string }[]).map(c => c.copy);

  it('takes a pair off the list, and Undo puts it back', () => {
    const before = listed();
    expect(before).toContain('notes/Plan 2.md');
    expect(conflictCopiesFixture('dismiss_conflict_copy', { copy: 'notes/Plan 2.md' })!.value).toBe('notes/Plan.md');
    expect(listed()).not.toContain('notes/Plan 2.md');
    expect(() => conflictCopiesFixture('dismiss_conflict_copy', { copy: 'notes/Plan 2.md' })).toThrow(/not a conflict copy/);
    conflictCopiesFixture('undismiss_conflict_copy', { copy: 'notes/Plan 2.md' });
    expect(listed()).toEqual(before);
  });

  it('the dialog offers it with Undo in its own note, and keeps Tab inside', () => {
    expect(DIALOG).toContain('onClick={() => void notACopy()}>Not a copy</Button>');
    expect(DIALOG).toContain("await api.dismissConflictCopy(copy);");
    expect(DIALOG).toContain("await api.undismissConflictCopy(copy);");
    expect(DIALOG).toContain('ccp-undo" onClick={() => void undo()()}>Undo</button>');
    expect(DIALOG).toContain("card?.querySelector<HTMLButtonElement>('.ccp-undo')?.focus()");
    // The focus trap (overnight B8) is the dialog's own: holdFocus on the card.
    expect(DIALOG).toContain('release = holdFocus(card);');
  });
});
