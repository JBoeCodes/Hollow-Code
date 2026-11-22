// Browser-mode fixtures for conflict copies (overnight A10): two pairs in
// memory per page load; a keep removes the pair, as the real resolve does,
// and Not a copy sets it aside until its Undo (morning M7).
import type { ConflictCopy, ConflictPair } from './generated/ipc';

const files: Record<string, string> = {
  'notes/Plan.md': '# Plan\n\n- [ ] Ship the release\n',
  'notes/Plan 2.md': '# Plan\n\n- [ ] Ship the release\n- [ ] Write the release note\n',
  'notes/Ideas.md': 'One idea.\n',
  'notes/Ideas (1).md': 'One idea.\n',
};
const notCopies: { copy: string; original: string }[] = [];
const copies: { copy: string; original: string }[] = [
  { copy: 'notes/Ideas (1).md', original: 'notes/Ideas.md' },
  { copy: 'notes/Plan 2.md', original: 'notes/Plan.md' },
];
const hash = (s: string) => `h${s.length}-${[...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7)}`;

export function conflictCopiesFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  switch (command) {
    case 'conflict_copies':
      return { value: copies.map((c): ConflictCopy => ({
        ...c, identical: files[c.copy] === files[c.original],
        copyModified: '2026-10-09T10:00:00Z', originalModified: '2026-10-08T09:00:00Z',
        copySize: files[c.copy].length, originalSize: files[c.original].length, originalProtected: false,
      })) };
    case 'conflict_copy_pair': {
      const c = copies.find(x => x.copy === args?.copy);
      if (!c) throw new Error(`${String(args?.copy)} is not a conflict copy with its original beside it.`);
      const pair: ConflictPair = { ...c, copyText: files[c.copy], originalText: files[c.original], copyHash: hash(files[c.copy]), originalHash: hash(files[c.original]) };
      return { value: pair };
    }
    case 'resolve_conflict_copy': {
      const i = copies.findIndex(x => x.copy === args?.copy);
      if (i < 0) throw new Error('not a conflict copy');
      const c = copies[i];
      if (args?.copyHash !== hash(files[c.copy]) || args?.originalHash !== hash(files[c.original])) throw new Error('One of the two files changed since you saw them. Nothing was moved; look again.');
      if (args?.keepCopy) files[c.original] = files[c.copy];
      delete files[c.copy];
      copies.splice(i, 1);
      (window as unknown as { __mockResolved?: unknown[] }).__mockResolved = [...((window as unknown as { __mockResolved?: unknown[] }).__mockResolved ?? []), { copy: c.copy, keepCopy: !!args?.keepCopy }];
      return { value: c.original };
    }
    case 'dismiss_conflict_copy': {
      const i = copies.findIndex(x => x.copy === args?.copy);
      if (i < 0) throw new Error(`${String(args?.copy)} is not a conflict copy with its original beside it.`);
      const [c] = copies.splice(i, 1);
      notCopies.push(c);
      return { value: c.original };
    }
    case 'undismiss_conflict_copy': {
      const i = notCopies.findIndex(x => x.copy === args?.copy);
      if (i < 0) throw new Error(`${String(args?.copy)} isn't marked as not a copy.`);
      const [c] = notCopies.splice(i, 1);
      copies.push(c);
      copies.sort((a, b) => a.copy.localeCompare(b.copy));
      return { value: null };
    }
    default: return null;
  }
}
