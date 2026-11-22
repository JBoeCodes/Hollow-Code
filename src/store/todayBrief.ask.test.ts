import { beforeEach, expect, it, vi } from 'vitest';

// Do it / Prepare (Today, the weekly review) open the Main agent page with
// the request in its composer, for the person to read, edit and send. Run 15
// P3 removed the Main agent drawer, so nothing goes into a box beside the
// page any more (P1-03's 'drawer' answer is gone).
const state = vi.hoisted(() => ({
  opened: 0, openOk: true,
  prefill: [] as (string | null)[], focus: 0, footer: [] as string[],
}));
vi.mock('../api', () => ({ api: {} }));
vi.mock('./runtime', () => ({
  tasksData: () => null, enableTasksResource: () => {}, commitmentsList: { latest: [] },
  enableCommitmentsResource: () => {}, refetchCommitments: () => {},
  setChatPrefillRequest: (t: string | null) => { state.prefill.push(t); },
  requestChatFocus: () => { state.focus++; },
  flashFooter: (t: string) => { state.footer.push(t); },
}));
vi.mock('./waitingOnYou', () => ({ waitingBadge: () => 0, waitingList: { items: () => [] } }));
vi.mock('./mainAgent', () => ({ openMainAgent: async () => { state.opened++; return state.openOk; } }));
import { askMainAgent } from './todayBrief';

beforeEach(() => {
  Object.assign(state, { opened: 0, openOk: true, prefill: [], focus: 0, footer: [] });
});

it('opens the COO page and fills its composer', async () => {
  expect(await askMainAgent('  Draft the agenda  ')).toBe('opened');
  expect(state.opened).toBe(1);
  expect(state.prefill).toEqual(['Draft the agenda']);
  expect(state.focus).toBe(1);
});

it('an empty request does nothing; a failed open says so and fills nothing', async () => {
  expect(await askMainAgent('   ')).toBe(false);
  expect(state.opened).toBe(0);
  state.openOk = false;
  expect(await askMainAgent('Draft the agenda')).toBe(false);
  expect(state.prefill).toEqual([]);
  expect(state.footer).toHaveLength(1);
});
