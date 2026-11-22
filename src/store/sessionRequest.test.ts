import { describe, expect, it } from 'vitest';
import { sessionRequestStep } from './sessionRequest';
import type { AgentSessionSummary } from '../types';

const summary = (key: string) => ({ key, title: key, agent: 'claude', cwd: '/vault' }) as AgentSessionSummary;

describe('sessionRequestStep (Chat opening a conversation by exact key, B·F15)', () => {
  it('opens the listed conversation when Chat lists it', () => {
    const listed = summary('s-1');
    expect(sessionRequestStep({ key: 's-1' }, [summary('s-0'), listed])).toEqual({ open: listed });
  });
  it("opens the handed-over summary when Chat's list doesn't carry the key", () => {
    const old = summary('s-old');
    expect(sessionRequestStep({ key: 's-old', session: old }, [summary('s-new')])).toEqual({ open: old });
  });
  it('looks up a key neither the request nor the loaded list carries, never a silent nothing', () => {
    expect(sessionRequestStep({ key: 's-gone' }, [summary('s-new')])).toEqual({ lookUp: 's-gone' });
    expect(sessionRequestStep({ key: 's-gone' }, [])).toEqual({ lookUp: 's-gone' });
  });
  it('waits for the list only when there is no summary', () => {
    expect(sessionRequestStep({ key: 's-1' }, undefined)).toBe('wait');
  });
  it('never opens past a running turn: says so, unless the running one is the one asked for', () => {
    const old = summary('s-old');
    // Another conversation's turn is on screen: openSession would do nothing.
    expect(sessionRequestStep({ key: 's-old', session: old }, [old], 's-running')).toBe('running');
    expect(sessionRequestStep({ key: 's-old' }, undefined, '')).toBe('running');
    expect(sessionRequestStep({ key: 's-running' }, [], 's-running')).toBe('shown');
    // Idle again: it opens.
    expect(sessionRequestStep({ key: 's-old', session: old }, [old], null)).toEqual({ open: old });
  });
});
