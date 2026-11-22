import { describe, expect, it } from 'vitest';
import { isRouteCall, routeChipLabel, routeReceipt } from './routeReceipt';

const RECEIPT = JSON.stringify({
  routed: true, to: 'coo', workspace: null, machine: { id: 'm-00000000000000aa', name: 'Mac mini' },
  session: 's-20261115-0000c000', state: 'queued', waitingFor: null, tell: 'Sent to Mac mini’s COO (queued).',
}, null, 2);

describe('route receipts (M3a follow-up)', () => {
  it('knows a route call by the action id, through Hollow’s run_action only', () => {
    expect(isRouteCall('mcp__hollow_app__run_action', { id: 'sessions.route_to_runner', args: { request: 'x' } })).toBe(true);
    expect(isRouteCall('mcp__hollow_app__run_action', { id: 'sessions.start' })).toBe(false);
    expect(isRouteCall('mcp__other__run_action', { id: 'sessions.route_to_runner' })).toBe(false);
    expect(isRouteCall('Bash', { id: 'sessions.route_to_runner' })).toBe(false);
  });

  it('reads where it went from the receipt, also with the broker’s note after it', () => {
    const r = { machineId: 'm-00000000000000aa', machine: 'Mac mini', session: 's-20261115-0000c000', to: 'coo' };
    expect(routeReceipt(RECEIPT)).toEqual(r);
    expect(routeReceipt(`${RECEIPT}\n\nHollow note: this ran in auto mode.`)).toEqual(r);
    expect(routeChipLabel(r as never)).toBe('Sent to Mac mini’s COO');
    const lead = routeReceipt(JSON.stringify({ routed: true, to: 'lead', machine: { id: 'm-1', name: '' }, session: 's-2' }));
    expect(lead).toEqual({ machineId: 'm-1', machine: 'the runner', session: 's-2', to: 'lead' });
    expect(routeChipLabel(lead!)).toBe('Sent to a lead on the runner');
  });

  it('has no chip while it runs, when it failed or when nothing was routed', () => {
    expect(routeReceipt(null)).toBeNull();
    expect(routeReceipt('Can’t reach Mac mini (ssh: timed out). Nothing was sent or queued.')).toBeNull();
    expect(routeReceipt(JSON.stringify({ routed: false, machine: { id: 'm-1' }, session: 's-2' }))).toBeNull();
    expect(routeReceipt(JSON.stringify({ routed: true, machine: { id: 'm-1' } }))).toBeNull();
  });
});
