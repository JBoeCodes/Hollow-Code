import { describe, expect, it } from 'vitest';
import {
  APPROVAL_BADGE_CAP,
  approvalBadgeLabel,
  badgeFromResolved,
  findToolById,
  rememberBadge,
  type ApprovalBadge,
} from './chatApproval';
import { approvalOriginLabel, approvalPlacement, approvalShownIn, approvalWaitingLabel } from './chatApprovalOrigin';

describe('findToolById', () => {
  const turns = [
    { tools: [{ id: 'toolu_a' }, { id: 'toolu_b' }] },
    { tools: [] },
    { tools: [{ id: 'toolu_c' }, { id: 'toolu_b' }] },
  ];

  it('finds the newest call with the id and nothing for an unknown id', () => {
    expect(findToolById(turns, 'toolu_b')).toEqual({ turnIdx: 2, toolIdx: 1 });
    expect(findToolById(turns, 'toolu_a')).toEqual({ turnIdx: 0, toolIdx: 0 });
    expect(findToolById(turns, 'toolu_zzz')).toBeNull();
  });

  it('never matches an unbound approval', () => {
    expect(findToolById(turns, null)).toBeNull();
    expect(findToolById(turns, undefined)).toBeNull();
    expect(findToolById([{ tools: [{}] }], '')).toBeNull();
  });
});

describe('badgeFromResolved', () => {
  it('says an iPhone answered (B4), and a window click stays plain', () => {
    const by = { kind: 'device' as const, deviceId: 'ph-0a1b2c3d', deviceName: 'iPhone', faceId: true, at: '2026-11-19T12:00:00Z', receipt: 'Allowed on iPhone with Face ID at 7:00 AM' };
    expect(badgeFromResolved({ approved: true, cancelled: false, reason: 'allowed', answeredBy: by })).toBe('allowed_phone');
    expect(badgeFromResolved({ approved: false, cancelled: false, reason: 'denied', answeredBy: { ...by, faceId: false } })).toBe('denied_phone');
    expect(badgeFromResolved({ approved: true, cancelled: false, reason: 'allowed', answeredBy: null })).toBe('allowed');
    expect(approvalBadgeLabel('allowed_phone')).toBe('Allowed on iPhone');
  });

  it('maps the reason', () => {
    expect(badgeFromResolved({ approved: true, cancelled: false, reason: 'allowed' })).toBe('allowed');
    expect(badgeFromResolved({ approved: false, cancelled: false, reason: 'denied' })).toBe('denied');
    expect(badgeFromResolved({ approved: false, cancelled: true, reason: 'timed_out' })).toBe('timed_out');
    expect(badgeFromResolved({ approved: false, cancelled: true, reason: 'cancelled' })).toBe('cancelled');
  });

  it('falls back to the legacy fields from an older backend', () => {
    expect(badgeFromResolved({ approved: true, cancelled: false })).toBe('allowed');
    expect(badgeFromResolved({ approved: false, cancelled: false })).toBe('denied');
    expect(badgeFromResolved({ approved: false, cancelled: true })).toBe('cancelled');
  });
});

describe('approvalBadgeLabel', () => {
  it('labels every state', () => {
    const labels: Record<ApprovalBadge, string> = {
      pending: 'Awaiting approval',
      allowed: 'Allowed',
      denied: 'Denied',
      timed_out: 'Timed out',
      cancelled: 'Cancelled',
      allowed_phone: 'Allowed on iPhone',
      denied_phone: 'Denied on iPhone',
    };
    for (const [badge, label] of Object.entries(labels)) {
      expect(approvalBadgeLabel(badge as ApprovalBadge)).toBe(label);
    }
  });
});

describe('rememberBadge', () => {
  it('keeps the newest state per id and drops the oldest past the cap', () => {
    const map = new Map<string, ApprovalBadge>();
    rememberBadge(map, 'toolu_1', 'pending');
    rememberBadge(map, 'toolu_1', 'denied');
    expect(map.get('toolu_1')).toBe('denied');
    for (let i = 2; i <= APPROVAL_BADGE_CAP + 1; i++) rememberBadge(map, `toolu_${i}`, 'pending');
    expect(map.size).toBe(APPROVAL_BADGE_CAP);
    expect(map.has('toolu_1')).toBe(false);
    expect(map.has(`toolu_${APPROVAL_BADGE_CAP + 1}`)).toBe(true);
  });
});

describe('subagent approvals in the parent chat (SA1)', () => {
  const planted = {
    sessionKey: 'sub-1',
    subagent: { key: 'sub-1', parentKey: 'parent-1', title: 'Release checker' },
  };
  it('shows in the subagent chat and in the chat that started it, not elsewhere', () => {
    expect(approvalShownIn(planted, new Set(['sub-1']))).toBe(true);
    expect(approvalShownIn(planted, new Set(['parent-1']))).toBe(true);
    expect(approvalShownIn(planted, new Set(['parent-0', 'parent-1']))).toBe(true);
    expect(approvalShownIn(planted, new Set(['other']))).toBe(false);
  });
  it('keeps the old rules for an ordinary or unbound card', () => {
    expect(approvalShownIn({ sessionKey: 'a' }, new Set(['a']))).toBe(true);
    expect(approvalShownIn({ sessionKey: 'a' }, new Set(['b']))).toBe(false);
    expect(approvalShownIn({ sessionKey: null }, new Set(['b']))).toBe(true);
  });
  it('names the subagent', () => {
    expect(approvalOriginLabel(planted)).toBe('From subagent: Release checker');
    expect(approvalOriginLabel({ subagent: { key: 'k', parentKey: 'p', title: ' ' } })).toBe('From a subagent');
    expect(approvalOriginLabel({})).toBeNull();
  });
});

describe('cards raised to the COO chat (2026-11-20)', () => {
  const routedTo = { askedKey: 'sub-1', askedTitle: 'Release checker', askedKind: 'subagent' as const, cooKey: 'coo-1' };
  const lead = { sessionKey: 'lead-1', subagent: null, routedTo: { askedKey: 'lead-1', askedKind: 'lead' as const, cooKey: 'coo-1' } };
  const sub = { sessionKey: 'sub-1', subagent: { key: 'sub-1', parentKey: 'lead-1', title: 'Release checker' }, routedTo };
  it('is a card only in the COO chat, whatever conversation that is', () => {
    for (const card of [lead, sub]) {
      expect(approvalPlacement(card, new Set(['coo-9']), true)).toBe('card');
      expect(approvalPlacement(card, new Set(), true)).toBe('card');
    }
  });
  it('is a one-line note in the chat it came from and in its parent, never a second card', () => {
    expect(approvalPlacement(lead, new Set(['lead-1']), false)).toBe('waiting');
    expect(approvalPlacement(sub, new Set(['sub-1']), false)).toBe('waiting');
    expect(approvalPlacement(sub, new Set(['lead-1']), false)).toBe('waiting');
    expect(approvalWaitingLabel(sub)).toBe('Waiting for approval in the COO chat');
    // Across every chat, exactly one place draws the buttons.
    for (const card of [lead, sub]) {
      const views: [Set<string>, boolean][] = [[new Set(['lead-1']), false], [new Set(['sub-1']), false], [new Set(['other']), false], [new Set(['coo-1']), true]];
      expect(views.filter(([keys, coo]) => approvalPlacement(card, keys, coo) === 'card')).toHaveLength(1);
    }
  });
  it('hides from unrelated chats, and leaves an unrouted card to the old rules', () => {
    expect(approvalPlacement(sub, new Set(['other']), false)).toBe('hidden');
    const planted = { sessionKey: 'sub-1', subagent: { key: 'sub-1', parentKey: 'parent-1', title: null } };
    expect(approvalPlacement(planted, new Set(['parent-1']), false)).toBe('card');
    expect(approvalPlacement(planted, new Set(['other']), false)).toBe('hidden');
    expect(approvalPlacement({ sessionKey: null }, new Set(['b']), false)).toBe('card');
    expect(approvalWaitingLabel(planted as never)).toBe('');
  });
});
