import { describe, expect, it } from 'vitest';
import rail from './components/Rail.tsx?raw';
import mission from './components/MissionControl.tsx?raw';
import home from './components/StudioHome.tsx?raw';
import work from './pages/Work.tsx?raw';
import orb from './components/MainAgentOrb.tsx?raw';
import css from './styles/session-indicator.css?raw';
import cssCheck from '../scripts/check-css-tokens.mjs?raw';
import states from './store/sessionStates.ts?raw';
import badge from './components/SessionIndicator.tsx?raw';
import { INTERRUPTED_HINT, MAIN_AGENT_STATE_WORDS, SESSION_STATE_ORDER, mainAgentStateOf, projectRollup, rollupSessions, sessionStateIndex, sessionStateLabel, sessionStateOf, stateOfRunStatus, type SessionEvidence } from './sessionIndicator';

const components = Object.keys(import.meta.glob('./components/*.tsx'));
const evidence = (over: Partial<SessionEvidence> = {}): SessionEvidence => ({
  onScreen: { key: null, busy: false }, background: [], queue: [], waitingKeys: [], feed: {}, seenFailures: new Map(), ...over,
});
const row = (id: number, key: string, status: string) => ({ id, source: 'chat', sourceId: key, status });

describe('session indicator states (asks 10-11 A2)', () => {
  it('orders the states most urgent first: waiting, failed, interrupted, running, queued, paused', () => {
    expect(SESSION_STATE_ORDER).toEqual(['waiting', 'failed', 'interrupted', 'running', 'queued', 'paused']);
    expect(sessionStateOf({ queued: true, paused: true })).toBe('queued');
    expect(sessionStateOf({ paused: true })).toBe('paused');
    expect(sessionStateOf({ failed: true, interrupted: true })).toBe('failed');
    expect(sessionStateOf({ interrupted: true, queued: true })).toBe('interrupted');
    expect(sessionStateOf({ waiting: true, failed: true, running: true, queued: true })).toBe('waiting');
    expect(sessionStateOf({ failed: true, running: true })).toBe('failed');
    expect(sessionStateOf({ running: true, queued: true })).toBe('running');
    expect(sessionStateOf({ queued: true })).toBe('queued');
    expect(sessionStateOf({})).toBe('idle');
  });

  it('names the provider in the label, not as the main signal', () => {
    expect(sessionStateLabel('running', 'Claude')).toBe('Running · Claude');
    expect(sessionStateLabel('waiting')).toBe('Waiting on you');
    expect(sessionStateLabel('idle', 'Codex')).toBe('Codex');
  });

  it('interrupted says what happened and what to do (P1-40), not "failed"', () => {
    expect(sessionStateLabel('interrupted')).toBe('Interrupted. Hollow quit before this turn finished. Open it and press Continue.');
    expect(sessionStateLabel('interrupted', 'Claude')).toBe('Interrupted · Claude. Hollow quit before this turn finished. Open it and press Continue.');
    expect(INTERRUPTED_HINT).toBe('Hollow quit before this turn finished. Open it and press Continue.');
    expect(badge).toContain("props.state === 'interrupted'");
    expect(css).toContain('.session-indicator[data-state="interrupted"]');
  });

  it('maps run statuses: live runs, queued waits, failures; finished ones are idle', () => {
    expect(['running', 'starting', 'claimed'].map(stateOfRunStatus)).toEqual(['running', 'running', 'running']);
    expect(stateOfRunStatus('queued')).toBe('queued');
    expect(stateOfRunStatus('failed')).toBe('failed');
    expect(stateOfRunStatus('interrupted')).toBe('interrupted');
    expect(['done', 'stopped', 'skipped', 'cancelled'].map(stateOfRunStatus)).toEqual(['idle', 'idle', 'idle', 'idle']);
  });
});

describe('every conversation’s state from live evidence', () => {
  it('reads running from the screen, the background and claimed queue rows; queued only without a running row', () => {
    const { states } = sessionStateIndex(evidence({
      onScreen: { key: 'a', busy: true }, background: ['b'],
      queue: [row(1, 'c', 'running'), row(2, 'd', 'queued'), row(3, 'c', 'queued'), { id: 4, source: 'cron', sourceId: 'e', status: 'running' }],
    }));
    expect(Object.fromEntries(states)).toEqual({ a: 'running', b: 'running', c: 'running', d: 'queued' });
  });

  it('a queued chat row that reads Working shows as running, never queued (2026-11-16)', () => {
    const { states } = sessionStateIndex(evidence({
      queue: [
        { id: 1, source: 'chat', sourceId: 'a', status: 'queued', working: true },
        { id: 2, source: 'chat', sourceId: 'b', status: 'queued' },
        { id: 3, source: 'chat', sourceId: 'c', status: 'claimed' },
      ],
    }));
    expect(Object.fromEntries(states)).toEqual({ a: 'running', b: 'queued', c: 'running' });
  });

  it('waiting beats running; an approval or open card on a session pauses it', () => {
    const { states } = sessionStateIndex(evidence({ background: ['a'], waitingKeys: ['a', null, undefined, 'z'] }));
    expect(states.get('a')).toBe('waiting');
    expect(states.get('z')).toBe('waiting');
    expect(states.size).toBe(2);
  });

  it('failed is the newest queue row failing; a newer row decides', () => {
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'done'), row(2, 'a', 'failed')] })).states.get('a')).toBe('failed');
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'failed'), row(2, 'a', 'done')] })).states.has('a')).toBe(false);
    // A failed turn being retried reads as running, not failed.
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'failed'), row(2, 'a', 'running')] })).states.get('a')).toBe('running');
  });

  it('a finished feed entry with an error fails; a later successful queue row supersedes it', () => {
    const feed = { a: { running: false, error: 'rate limit', finishedAt: 5 }, b: { running: true, error: 'x', finishedAt: null }, c: { running: false, error: 'boom', finishedAt: 7 } };
    const { states, failures } = sessionStateIndex(evidence({ feed, queue: [row(9, 'c', 'done')] }));
    expect(states.get('a')).toBe('failed');
    expect(failures.get('a')).toBe('f:5');
    expect(states.has('c')).toBe(false);
    expect(states.has('b')).toBe(false);
  });

  it('a feed error defers to its own queue row (review: a seen failure must not come back)', () => {
    const feed = (queueId: number) => ({ a: { running: false, error: 'mid-turn hiccup', finishedAt: 9, queueId } });
    // Its row finished fine: a recoverable error, not a failure.
    expect(sessionStateIndex(evidence({ feed: feed(3), queue: [row(3, 'a', 'done')] })).states.has('a')).toBe(false);
    // A newer turn came after it.
    expect(sessionStateIndex(evidence({ feed: feed(3), queue: [row(5, 'a', 'done')] })).states.has('a')).toBe(false);
    // Its row aged out of the queue's short history: the same q: token, so a
    // failure seen while the row was listed stays seen.
    const aged = sessionStateIndex(evidence({ feed: feed(3), seenFailures: new Map([['a', 'q:3']]) }));
    expect(aged.failures.get('a')).toBe('q:3');
    expect(aged.states.has('a')).toBe(false);
    expect(sessionStateIndex(evidence({ feed: feed(3) })).states.get('a')).toBe('failed');
  });

  it('paused is its own state (P1a): not running, not failed, and a turn starting wins', () => {
    expect(sessionStateIndex(evidence({ paused: ['a'] })).states.get('a')).toBe('paused');
    // Its row ended cancelled; an older failure token doesn't make it failed.
    expect(sessionStateIndex(evidence({ paused: ['a'], queue: [row(1, 'a', 'failed')] })).states.get('a')).toBe('paused');
    expect(sessionStateIndex(evidence({ paused: ['a'], queue: [row(2, 'a', 'running')] })).states.get('a')).toBe('running');
    expect(sessionStateIndex(evidence({ paused: ['a'], onScreen: { key: 'a', busy: true } })).states.get('a')).toBe('running');
    expect(sessionStateLabel('paused', 'Claude')).toBe('Paused · Claude');
    expect(rollupSessions([{ title: 'Memo', state: 'paused' }, { title: 'Plan', state: 'running' }])?.label).toBe('1 running: Plan · 1 paused: Memo');
    expect(badge).toContain("props.state === 'paused'");
    expect(states).toContain('paused: Object.keys(pausedChats())');
  });

  it('interrupted is its own state (P1-40): the newest row interrupted, or a feed error from the quit; opening it marks it seen', () => {
    const one = sessionStateIndex(evidence({ queue: [row(1, 'a', 'interrupted')] }));
    expect(one.states.get('a')).toBe('interrupted');
    expect(one.failures.get('a'), 'opening it marks it seen like a failure').toBe('q:1');
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'interrupted')], seenFailures: new Map([['a', 'q:1']]) })).states.has('a')).toBe(false);
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'interrupted'), row(2, 'a', 'running')] })).states.get('a')).toBe('running');
    expect(sessionStateIndex(evidence({ queue: [row(1, 'a', 'interrupted'), row(2, 'a', 'failed')] })).states.get('a')).toBe('failed');
    // Its row aged out of the queue's short history: the quit's own words still say interrupted.
    const feed = { a: { running: false, error: 'Hollow stopped before this run finished', finishedAt: 9, queueId: 3 }, b: { running: false, error: 'rate limit', finishedAt: 9, queueId: 4 } };
    const aged = sessionStateIndex(evidence({ feed }));
    expect(aged.states.get('a')).toBe('interrupted');
    expect(aged.states.get('b')).toBe('failed');
  });

  it('a failure the person has looked at no longer shows; a newer failure does', () => {
    const queue = [row(4, 'a', 'failed')];
    expect(sessionStateIndex(evidence({ queue, seenFailures: new Map([['a', 'q:4']]) })).states.has('a')).toBe(false);
    expect(sessionStateIndex(evidence({ queue: [...queue, row(6, 'a', 'failed')], seenFailures: new Map([['a', 'q:4']]) })).states.get('a')).toBe('failed');
  });
});

describe('a collapsed workspace’s roll-up (asks 10-11 A1)', () => {
  it('shows nothing when every conversation is idle', () => {
    expect(rollupSessions([])).toBeNull();
    expect(rollupSessions([{ title: 'a', state: 'idle' }])).toBeNull();
  });

  it('the most urgent state wins, counts that state and names every live chat by state', () => {
    const r = rollupSessions([
      { title: 'Memo', state: 'running' }, { title: 'Pricing', state: 'running' }, { title: 'Receipts', state: 'waiting' },
      { title: 'Old', state: 'idle' }, { title: '', state: 'queued' },
    ]);
    expect(r).toEqual({ state: 'waiting', count: 1, label: '1 waiting on you: Receipts · 2 running: Memo, Pricing · 1 queued: Conversation' });
  });

  it('counts past one and cuts long name lists', () => {
    const r = rollupSessions(['a', 'b', 'c', 'd', 'e'].map(title => ({ title, state: 'running' as const })), 3);
    expect(r?.count).toBe(5);
    expect(r?.label).toBe('5 running: a, b, c +2 more');
  });
});

describe('the COO’s variant', () => {
  it('waiting on you beats working beats paused beats watching beats resting', () => {
    expect(mainAgentStateOf({ waiting: true, working: true, paused: true, watching: true })).toBe('waiting');
    expect(mainAgentStateOf({ working: true, paused: true })).toBe('working');
    expect(mainAgentStateOf({ paused: true, watching: true })).toBe('paused');
    expect(mainAgentStateOf({ watching: true })).toBe('watching');
    expect(mainAgentStateOf({})).toBe('resting');
    expect(MAIN_AGENT_STATE_WORDS.paused).toBe('Paused by the cost cap');
  });
});

describe('one component everywhere', () => {
  it('every surface draws state through SessionIndicator, and the light trails are gone', () => {
    expect(components).not.toContain('./components/RunningSessionMark.tsx');
    expect(components).toContain('./components/SessionIndicator.tsx');
    for (const [file, text, needle] of [
      ['Rail', rail, '<SessionRollupBadge '],
      ['MissionControl', mission, '<SessionIndicator '],
      ['MissionControl', mission, '<SessionRollupBadge '],
      ['StudioHome', home, '<SessionRollupBadge '],
      ['Work', work, '<SessionIndicator '],
      ['MainAgentOrb', orb, '<MainAgentIndicator '],
    ] as const) expect(text, `${file} uses ${needle}`).toContain(needle);
    // L8: a project row lists no chats, so its badge always rolls up the
    // project's live states (its lead's and its other chats').
    expect(rail).toContain('const rollup = () => workspaceRollup(w().id);');
  });

  it('marks stay put while a turn streams, and a failure is seen only on screen (review)', () => {
    // A streamed token changes the feed, not any state: the memo compares content.
    expect(states).toContain('{ equals: sameIndex }');
    // Not keyed: a changed roll-up updates the badge in place.
    expect(badge).toContain('<Show when={props.rollup}>{r =>');
    expect(work).toContain('<Show when={state()}>{r =>');
    expect(states).toContain("const onScreen = (activePage() === 'today' && !!workspaceId()) || conversationHistoryShown();");
    // Seen failures survive a restart.
    expect(states).toContain("const SEEN_KEY = 'hollow.seenFailures';");
  });

  it('only the running arc moves, gated by both reduced-motion switches and paused while the window is away', () => {
    const at = css.indexOf('@media (prefers-reduced-motion: no-preference)');
    expect(at).toBeGreaterThan(0);
    expect(css.slice(0, at)).not.toMatch(/animation\s*:/);
    const lines = css.slice(at).split('\n').filter((l: string) => /animation\s*:/.test(l));
    expect(lines).toHaveLength(1);
    expect(lines[0]).toContain(':root:not([data-motion="reduced"]) .session-indicator-arc');
    expect(lines[0]).toContain('animation-play-state: var(--ambient-play, running)');
    // It is no longer exempt from the pause switch.
    expect(cssCheck).not.toContain('rail-session-orbit');
  });
});


describe('COO expressive states (V3)', () => {
  it('keeps an own failure visible unless an own conversation waits', () => {
    expect(mainAgentStateOf({ failed: true, working: true })).toBe('failed');
    expect(mainAgentStateOf({ waiting: true, failed: true, night: true })).toBe('waiting');
  });
  it('night needs running queue evidence and does not override attention', () => {
    expect(mainAgentStateOf({ night: true, working: true })).toBe('night');
    expect(mainAgentStateOf({ night: true, failed: true })).toBe('failed');
    expect(mainAgentStateOf({ paused: true })).toBe('paused');
  });
});

describe('project row running indicator (11-11)', () => {
  const e = (state: 'running' | 'queued' | 'waiting' | 'idle', role: 'lead' | 'subagent' | 'chat') => ({ title: 't', state, role });
  it('says nothing when every session is idle', () => {
    expect(projectRollup([])).toBeNull();
    expect(projectRollup([e('idle', 'lead'), e('idle', 'subagent')])).toBeNull();
  });
  it('names the lead and its subagents when they run', () => {
    expect(projectRollup([e('running', 'lead'), e('running', 'subagent'), e('running', 'subagent'), e('idle', 'chat')]))
      .toEqual({ state: 'running', count: 3, label: 'Lead and 2 subagents running' });
  });
  it('counts a project with only a subagent running', () => {
    expect(projectRollup([e('idle', 'lead'), e('running', 'subagent')])).toEqual({ state: 'running', count: 1, label: '1 subagent running' });
  });
  it('shows queued work, and other chats in the project', () => {
    expect(projectRollup([e('queued', 'lead')])).toEqual({ state: 'queued', count: 1, label: 'Lead queued' });
    expect(projectRollup([e('running', 'chat'), e('queued', 'subagent')])).toEqual({ state: 'running', count: 1, label: '1 chat running · 1 subagent queued' });
  });
  it('shows waiting on you first, as the COO orb does', () => {
    const r = projectRollup([e('running', 'subagent'), e('waiting', 'lead')]);
    expect(r).toEqual({ state: 'waiting', count: 1, label: 'Lead waiting on you · 1 subagent running' });
  });
  it('is the one roll-up the store builds, with subagents counted and the lead found by its key', () => {
    expect(states).toContain('projectRollup(entries)');
    expect(states).toContain('projectAgentKey');
    expect(states).not.toMatch(/forkKind !== 'subagent'/);
    expect(rail).not.toContain('railListed');
  });
});
