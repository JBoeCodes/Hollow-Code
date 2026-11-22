import { describe, expect, it } from 'vitest';
import { layoutDay, type AgendaEvent } from './agenda';
import { ringPoint, ringArc, ringNow, ringItems, ringCalendarLabel, calendarNeedsSignIn, CALENDAR_SIGN_IN_LABEL } from './dayRing';
import m365AuthSource from '../src-tauri/src/m365_auth.rs?raw';
const event = (start: string, end: string, extra = {}): AgendaEvent => ({ id: start, title: 'Meeting', start, end, allDay: false, cancelled: false, ...extra });
describe('day ring', () => {
  it('names Off and Mac permission states without Microsoft sign-in advice', () => {
    for (const status of ['Calendar is off', 'Calendar access is denied']) {
      expect(ringCalendarLabel({ connected: false, events: [], fetchedAt: '', error: null, source: 'Calendar', status })).toBe(status);
    }
  });
  it('puts midnight at the top and noon at the bottom', () => {
    expect(ringPoint(0, 1440, 100)).toEqual({ x: 180, y: 80 });
    expect(ringPoint(720, 1440, 100).y).toBeCloseTo(280);
    expect(ringArc(0, 1440, 1440, 100).match(/ A /g)).toHaveLength(2);
  });
  it('uses real DST day length and distinguishes repeated hours', () => {
    const l = layoutDay('2026-11-01', [event('2026-11-01T01:30:00-04:00','2026-11-01T01:45:00-04:00'),event('2026-11-01T01:30:00-05:00','2026-11-01T01:45:00-05:00')], [], []);
    expect(l.minutes).toBe(1500);
    const items = ringItems(l);
    expect(items[0].detail).not.toBe(items[1].detail);
    expect(items[0].path).not.toBe(items[1].path);
    expect(ringNow(l, new Date('2026-11-01T01:30:00-05:00'))).toBe(150);
    expect(layoutDay('2026-03-08', [], [], []).minutes).toBe(1380);
  });
  it('keeps all-day and untimed items outside the dial, retaining cancelled events', () => {
    const l = layoutDay('2026-10-12', [event('2026-10-12','2026-10-13',{allDay:true}), event('2026-10-12T09:00:00-04:00','2026-10-12T10:00:00-04:00',{cancelled:true})], [], [{url:'t',title:'Due',due:'2026-10-12',dueTime:null,durationMin:null,status:'Open'}]);
    expect(ringItems(l)).toHaveLength(1);
    expect(ringItems(l)[0].detail).toContain('cancelled');
    expect(l.allDay).toHaveLength(1); expect(l.dueNoTime).toHaveLength(1);
    expect(ringNow(l, new Date('2026-10-13T00:00:00-04:00'))).toBeNull();
  });
  it('keeps a zero-duration reminder as a timed mark and list entry', () => {
    const l = layoutDay('2026-10-12', [event('2026-10-12T09:00:00-04:00','2026-10-12T09:00:00-04:00')], [], []);
    expect(ringItems(l)).toHaveLength(1);
    expect(ringItems(l)[0].path).toContain(' L ');
  });
  it('retains an invalid timed item in the list with an explicit missing time', () => {
    const l = layoutDay('2026-10-12', [], [], [{url:'bad',title:'Bad time',due:'2026-10-12',dueTime:'oops',durationMin:null,status:'Open'}]);
    expect(ringItems(l)).toHaveLength(1);
    expect(ringItems(l)[0].detail).toContain('Time unavailable');
    expect(ringItems(l)[0].path).toBe('');
  });
  it('uses actual short meeting durations, not the timeline minimum hit height', () => {
    const l = layoutDay('2026-10-12', [event('2026-10-12T09:00:00-04:00','2026-10-12T09:05:00-04:00')], [], []);
    expect(ringItems(l)[0].detail).toContain('9:05');
    expect(ringItems(l)[0].path).toBe(ringArc(540,545,1440,126));
  });
  it('does not represent unavailable or partial calendars as a free day', () => {
    expect(ringCalendarLabel(null)).toContain('unavailable');
    expect(ringCalendarLabel({connected:false,events:[],fetchedAt:'',error:null,source:'Calendar'})).toContain('not connected');
    expect(ringCalendarLabel({connected:true,events:[],fetchedAt:'',error:null,source:'Calendar',complete:false})).toContain('partial');
  });
  it('says a signed-out Microsoft 365 calendar needs signing in again (overnight H6)', () => {
    const refused = 'Microsoft 365 needs signing in again in Hollow → Settings → Connectors (AADSTS50173)';
    const expired = 'Microsoft 365 sign-in expired: sign in again in Hollow → Settings → Connectors';
    for (const error of [refused, expired, `couldn't read the calendar: ${refused}`]) {
      expect(calendarNeedsSignIn(error)).toBe(true);
      expect(ringCalendarLabel({connected:true,events:[],fetchedAt:'',error,source:'Microsoft 365',complete:false})).toBe(CALENDAR_SIGN_IN_LABEL);
    }
    // Anything else stays "unavailable" (a retry may help).
    for (const error of ['couldn\'t reach Microsoft: timed out', 'couldn\'t refresh the Microsoft sign-in (Microsoft answered 503: )', '', null, undefined]) expect(calendarNeedsSignIn(error)).toBe(false);
    expect(ringCalendarLabel({connected:true,events:[],fetchedAt:'',error:'Microsoft Graph 503',source:'Microsoft 365'})).toBe('Calendar unavailable');
    expect(CALENDAR_SIGN_IN_LABEL).toBe('Microsoft 365 needs you to sign in again');
  });
  it('matches the backend’s sign-in messages (m365_auth.rs)', () => {
    // There is no error code; a reworded message must fail here.
    expect(m365AuthSource).toContain('const NEEDS_SIGN_IN: &str = "Microsoft 365 needs signing in again";');
    expect(m365AuthSource).toContain('const EXPIRED: &str = "Microsoft 365 sign-in expired";');
  });
});
