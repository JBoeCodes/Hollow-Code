import { describe, expect, it } from 'vitest';
import { cancelledNotice, pausedNotice, pausedRowsToProbe, RESUME_PROMPT } from './chatStop';
import CHAT_PAUSE_RS from '../src-tauri/src/chat_pause.rs?raw';
import CHAT_STOP_SOURCE from './chatStop.ts?raw';

describe('cancelledNotice', () => {
  it('renders neutral stop text', () => {
    expect(cancelledNotice('verified')).toBe('Stopped.');
    expect(cancelledNotice(null)).toBe('Stopped.');
    expect(cancelledNotice(undefined)).toBe('Stopped.');
    expect(cancelledNotice('unconfirmed')).toBe(
      'Stopped. Hollow could not confirm that every process from this turn exited.',
    );
  });

  it('never reads like an error', () => {
    for (const stop of ['verified', 'unconfirmed', null, undefined] as const) {
      expect(cancelledNotice(stop)).not.toMatch(/error|fail/i);
    }
  });

  it('a paused turn says what it was doing and that Resume picks it up, never an error (P1a)', () => {
    expect(pausedNotice('Reading the plan.')).toBe('Paused: Reading the plan. Resume picks it up from here.');
    expect(pausedNotice('  ')).toBe('Paused. Resume picks it up from here.');
    expect(pausedNotice(null)).toBe('Paused. Resume picks it up from here.');
    expect(pausedNotice('Was using Bash')).not.toMatch(/error|fail|stopped/i);
  });

  it('the resume bubble shows the words the backend sends (chat_pause::RESUME_PROMPT)', () => {
    expect(CHAT_PAUSE_RS).toContain(`pub(crate) const RESUME_PROMPT: &str = "${RESUME_PROMPT}";`);
  });

  it('probes each chat once, by its newest row, only when that row ended cancelled (P1a)', () => {
    const rows = [
      { id: 1, source: 'chat', sourceId: 'a', status: 'cancelled' },
      { id: 2, source: 'chat', sourceId: 'a', status: 'done' },
      { id: 3, source: 'chat', sourceId: 'b', status: 'cancelled' },
      { id: 4, source: 'cron', sourceId: 'c', status: 'cancelled' },
      { id: 5, source: 'chat', sourceId: null, status: 'cancelled' },
    ];
    expect(pausedRowsToProbe(rows, new Set())).toEqual([{ id: 3, key: 'b' }]);
    expect(pausedRowsToProbe(rows, new Set([3]))).toEqual([]);
  });

  it('imports nothing from the store or Tauri', () => {
    expect(CHAT_STOP_SOURCE).not.toMatch(/^\s*import\s/m);
  });
});
