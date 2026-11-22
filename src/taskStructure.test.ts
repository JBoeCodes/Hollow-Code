import { describe, expect, it } from 'vitest';
import {
  blockerCandidates, blockerLabel, blockerWarningText, isCancelledStatus, wouldCycle, type BlockerTask,
} from './taskStructure';

function t(slug: string, over: Partial<BlockerTask> = {}): BlockerTask {
  return {
    url: `vault:tasks/${slug}.md`,
    title: slug.toUpperCase(),
    status: 'Not started',
    recurrence: null,
    blockedBy: [],
    ...over,
  };
}

describe('blocker candidates, cycle guard and warnings (Paperclip PR 11b)', () => {
  it('detects 1-, 2- and 3-cycles with case-insensitive URLs', () => {
    const a = t('a', { blockedBy: ['vault:tasks/B.md'] });
    const b = t('b', { blockedBy: ['vault:tasks/c.md'] });
    const c = t('c');
    const tasks = [a, b, c];
    expect(wouldCycle(tasks, a.url, a.url)).toBe(true); // 1: itself
    expect(wouldCycle(tasks, b.url, a.url)).toBe(true); // 2: a waits on b
    expect(wouldCycle(tasks, c.url, a.url)).toBe(true); // 3: a → b → c
    expect(wouldCycle(tasks, a.url, c.url)).toBe(false);
    expect(wouldCycle(tasks, 'VAULT:TASKS/C.MD', 'vault:tasks/a.md')).toBe(true);
  });

  it('exclude self, chosen, finished and loop-closing tasks; recurring and cancelled sort last with a note', () => {
    const self = t('self');
    const waitsOnSelf = t('loop', { blockedBy: [self.url] });
    const tasks = [
      self,
      waitsOnSelf,
      t('open'),
      t('chosen'),
      t('finished', { status: 'Done' }),
      t('weekly', { recurrence: 'every week' }),
      t('dropped', { status: 'Won’t do' }),
      t('alpha'),
    ];
    const got = blockerCandidates(tasks, self.url, ['vault:tasks/CHOSEN.md']);
    expect(got.map(c => c.task.title)).toEqual(['ALPHA', 'OPEN', 'DROPPED', 'WEEKLY']);
    expect(got[0].note).toBeNull();
    expect(got[2].note).toContain('cancelled');
    expect(got[3].note).toContain('repeats');
    // A new task (no URL yet) can wait on anything open, loops included.
    expect(blockerCandidates(tasks, null, []).map(c => c.task.title)).toContain('LOOP');
    // The query filters titles; the limit caps the list.
    expect(blockerCandidates(tasks, self.url, [], 'alp').map(c => c.task.title)).toEqual(['ALPHA']);
    expect(blockerCandidates(tasks, self.url, [], '', 2)).toHaveLength(2);
  });

  it('write all four warning kinds', () => {
    expect(blockerWarningText({ url: 'vault:tasks/gone.md', kind: 'missing' }, 'gone'))
      .toBe('No task at tasks/gone.md (renamed, moved or deleted), so this blocker will never resolve.');
    expect(blockerWarningText({ url: 'vault:tasks/x.md', kind: 'cancelled' }, 'Ship it')).toContain('Ship it is cancelled');
    expect(blockerWarningText({ url: 'vault:tasks/x.md', kind: 'recurring' }, 'Weekly')).toContain('Weekly repeats');
    expect(blockerWarningText({ url: 'vault:tasks/x.md', kind: 'cycle' }, 'Review')).toContain('neither can start');
  });

  it('label blockers by title, else by file name', () => {
    const tasks = [t('a')];
    expect(blockerLabel(tasks, 'vault:tasks/A.md')).toBe('A');
    expect(blockerLabel(tasks, 'vault:tasks/gone-away.md')).toBe('gone-away');
  });

  it('mirror the backend cancelled vocabulary', () => {
    for (const s of ['Cancelled', 'canceled', 'Won’t do', "won't  do", 'WONT DO']) expect(isCancelledStatus(s)).toBe(true);
    for (const s of ['Cancel later', 'Done', '']) expect(isCancelledStatus(s)).toBe(false);
  });
});
