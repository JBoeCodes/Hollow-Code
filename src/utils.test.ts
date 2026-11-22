import { describe, expect, it } from 'vitest';
import {
  dueLabel,
  errorWithPrefix,
  fmtClockHHMM,
  hueFor,
  localDateKey,
  parseClockTime,
  parseFrontmatter,
  stripFrontmatter,
  taskErrorText,
} from './utils';

describe('parseClockTime', () => {
  it('parses 12-hour forms', () => {
    expect(parseClockTime('4pm')).toBe('16:00');
    expect(parseClockTime('4 pm')).toBe('16:00');
    expect(parseClockTime('11:30am')).toBe('11:30');
    expect(parseClockTime('11.30 PM')).toBe('23:30');
    expect(parseClockTime('12am')).toBe('00:00');
    expect(parseClockTime('12pm')).toBe('12:00');
  });

  it('parses 24-hour and bare-digit forms', () => {
    expect(parseClockTime('16:00')).toBe('16:00');
    expect(parseClockTime('9.30')).toBe('09:30');
    expect(parseClockTime('930')).toBe('09:30');
    expect(parseClockTime('1630')).toBe('16:30');
    expect(parseClockTime('11')).toBe('11:00');
  });

  it('parses noon and midnight', () => {
    expect(parseClockTime('noon')).toBe('12:00');
    expect(parseClockTime('midnight')).toBe('00:00');
  });

  it('rejects unreadable input', () => {
    expect(parseClockTime('')).toBeNull();
    expect(parseClockTime('25:00')).toBeNull();
    expect(parseClockTime('13pm')).toBeNull();
    expect(parseClockTime('9:75')).toBeNull();
    expect(parseClockTime('soonish')).toBeNull();
  });
});

describe('fmtClockHHMM', () => {
  it('renders canonical HH:MM as compact 12-hour', () => {
    expect(fmtClockHHMM('14:30')).toBe('2:30pm');
    expect(fmtClockHHMM('00:05')).toBe('12:05am');
    expect(fmtClockHHMM('12:00')).toBe('12:00pm');
  });

  it('passes oddities through unchanged', () => {
    expect(fmtClockHHMM('whenever')).toBe('whenever');
    expect(fmtClockHHMM('25:99')).toBe('25:99');
  });
});

describe('dueLabel', () => {
  it('never calls completed work late', () => {
    expect(dueLabel({ due: '2026-05-01', daysUntilDue: -34, bucket: 'done' })).toBe('done');
    expect(dueLabel({ due: '2026-05-01', daysUntilDue: -34, bucket: 'done' }, true)).toBe('done');
    expect(dueLabel({ due: null, daysUntilDue: null, bucket: 'done' })).toBe('—');
    expect(dueLabel({ due: null, daysUntilDue: null, bucket: 'done' }, true)).toBe('');
  });

  it('renders the long form by day delta', () => {
    expect(dueLabel({ due: '2026-06-10', daysUntilDue: -3, bucket: 'overdue' })).toBe('3d overdue');
    expect(dueLabel({ due: '2026-06-13', daysUntilDue: 0, bucket: 'today' })).toBe('due today');
    expect(dueLabel({ due: '2026-06-14', daysUntilDue: 1, bucket: 'thisweek' })).toBe('due tomorrow');
    expect(dueLabel({ due: '2026-06-18', daysUntilDue: 5, bucket: 'thisweek' })).toBe('due in 5d');
    expect(dueLabel({ due: null, daysUntilDue: null, bucket: 'undated' })).toBe('—');
  });

  it('renders the compact form off the bucket', () => {
    expect(dueLabel({ due: '2026-06-12', daysUntilDue: -1, bucket: 'overdue' }, true)).toBe('overdue');
    expect(dueLabel({ due: '2026-06-01', daysUntilDue: -12, bucket: 'overdue' }, true)).toBe('12d late');
    expect(dueLabel({ due: '2026-06-13', daysUntilDue: 0, bucket: 'today' }, true)).toBe('today');
    expect(dueLabel({ due: null, daysUntilDue: null, bucket: 'undated' }, true)).toBe('');
  });
});

describe('hueFor', () => {
  it('is stable and in range', () => {
    expect(hueFor('alice')).toBe(hueFor('alice'));
    for (const s of ['a', 'bob', 'alex.morgan', '']) {
      const h = hueFor(s);
      expect(h).toBeGreaterThanOrEqual(0);
      expect(h).toBeLessThan(360);
    }
  });
});

describe('local date helpers', () => {
  it('localDateKey reads the local calendar day', () => {
    expect(localDateKey(new Date(2026, 0, 2))).toBe('2026-01-02');
  });
});

describe('parseFrontmatter', () => {
  it('parses scalars and lists', () => {
    const doc = '---\ntitle: Hello\ncount: 3\ndone: true\ntags:\n  - a\n  - b\n---\nbody here';
    const fm = parseFrontmatter(doc);
    expect(fm).not.toBeNull();
    expect(fm!.props.title).toBe('Hello');
    expect(fm!.props.count).toBe(3);
    expect(fm!.props.done).toBe(true);
    expect(fm!.props.tags).toEqual(['a', 'b']);
    expect(doc.slice(fm!.bodyStart)).toBe('body here');
  });

  it('returns null without a frontmatter block', () => {
    expect(parseFrontmatter('# just a doc')).toBeNull();
  });
});

describe('stripFrontmatter', () => {
  it('drops a well-formed block', () => {
    expect(stripFrontmatter('---\na: 1\n---\nbody')).toBe('body');
  });
  it('leaves bodies without frontmatter alone', () => {
    expect(stripFrontmatter('plain text')).toBe('plain text');
  });
});

describe('taskErrorText', () => {
  it('strips task error codes and accepts string rejections', () => {
    // Tauri rejects invoke() with a plain string, not an Error.
    expect(taskErrorText('TASK_CONFLICT: x changed')).toBe('x changed');
    expect(taskErrorText(new Error('TASK_LINKED: y'))).toBe('y');
    expect(taskErrorText('plain')).toBe('plain');
    expect(taskErrorText('TASK_KEY: the idempotency key is too long')).toBe('the idempotency key is too long');
    expect(taskErrorText('TASK_CYCLE: A → B → A. Nothing was saved')).toBe('A → B → A. Nothing was saved');
    expect(taskErrorText('TASK_BLOCKER: no task exists at vault:tasks/x.md')).toBe('no task exists at vault:tasks/x.md');
  });
});

describe('errorWithPrefix', () => {
  it('returns the message after a matching prefix, else null', () => {
    expect(errorWithPrefix('TASK_KEY: too long', 'TASK_KEY')).toBe('too long');
    expect(errorWithPrefix(new Error('TASK_KEY:tight'), 'TASK_KEY')).toBe('tight');
    expect(errorWithPrefix('TASK_CONFLICT: x', 'TASK_KEY')).toBeNull();
    // Only a leading prefix counts.
    expect(errorWithPrefix('failed: TASK_KEY: x', 'TASK_KEY')).toBeNull();
    expect(errorWithPrefix(undefined, 'TASK_KEY')).toBeNull();
  });
});
