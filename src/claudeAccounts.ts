/**
 * Pure helpers for Settings, Claude accounts. Type-only imports, so the
 * node vitest suite can load it.
 */
import type { ClaudeAccount, ClaudeAccountStatus, ClaudeAccountWindow, ClaudeAccountsView } from './types';

export const STATUS_LABELS: Record<ClaudeAccountStatus, string> = {
  ok: 'Has room',
  near: 'Near its limit',
  limited: 'Limit reached',
  invalid: 'Token refused',
  missingToken: 'Token missing',
  unknown: 'Not measured yet',
};

/** "42%", "7.5%", "101%": over 100 is kept (overage), never clamped. */
export function formatPercent(n: number): string {
  const shown = n >= 10 || Number.isInteger(n) ? String(Math.round(n)) : n.toFixed(1);
  return `${shown}%`;
}

/** The meter fill only, clamped to the track (0..1). */
export function fillFraction(n: number): number {
  return Math.min(Math.max(n / 100, 0), 1);
}

export function windowLevel(w: ClaudeAccountWindow | null, ceiling: number): 'ok' | 'high' | 'over' {
  if (!w) return 'ok';
  if (w.usedPercent >= 100) return 'over';
  if (w.usedPercent >= ceiling) return 'high';
  return 'ok';
}

/** "Resets Mon 3:00 PM", or null. */
export function resetLabel(iso: string | null, now: Date = new Date()): string | null {
  if (!iso) return null;
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return null;
  const minutes = Math.round((at.getTime() - now.getTime()) / 60_000);
  if (minutes <= 0) return 'Resetting now';
  if (minutes < 60) return `Resets in ${minutes} min`;
  if (minutes < 24 * 60) return `Resets in ${Math.round(minutes / 60)} h`;
  return `Resets ${at.toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' })}`;
}

export function sourceLabel(source: 'probe' | 'stream'): string {
  return source === 'probe' ? 'checked directly' : 'from its last run';
}

/** Spread usage across accounts (2026-10-21): what puts every Claude chat on Claude
 *  Code's own login whatever the routing (`agent.rs` → `apply_login`): the
 *  claude.ai connectors set to Always for chats (names), and whose login
 *  that is (`jo@x.com in Acme`, null when unknown). Null when nothing does. */
export interface LoginPin { connectors: string[]; login: string | null }

/** "A", "A and B", "A, B and C". */
export function joinNames(names: string[]): string {
  return names.length <= 1 ? names.join('') : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** One line describing which account the next run uses, and why. With a
 *  `pin`, chats don't use the routed account, except (while the login is
 *  out of room, [`loginOutOfRoom`]) the ones that don't need the connector. */
export function routingSummary(view: ClaudeAccountsView, pin: LoginPin | null = null): string {
  if (view.accounts.length === 0) return 'Every Claude run uses your Claude Code login. Add accounts to choose or spread across them.';
  const next = view.accounts.find(a => a.id === view.nextAccount);
  if (!next) return 'Waiting for an eligible Claude account to have room.';
  const name = next.label;
  if (pin?.connectors.length) {
    const who = pin.login ? ` (${pin.login})` : '';
    const one = pin.connectors.length === 1;
    const names = joinNames(pin.connectors);
    const nextWord = view.routing === 'spread' ? ' next' : '';
    if (loginOutOfRoom(view)) {
      return `Claude Code's login${who} is out of room, so ${names} (set to Always) ${one ? 'comes' : 'come'} along only when a message needs ${one ? 'it' : 'one'}. Other chats use ${name}${nextWord}.`;
    }
    return `Chats run on Claude Code's login${who}, not on your added accounts, because ${names} ${one ? 'is' : 'are'} set to Always. Runs without a claude.ai connector use ${name}${nextWord}.`;
  }
  const inRotation = view.accounts.filter(a => a.enabled && a.status !== 'invalid' && a.status !== 'missingToken');
  return `Automatically choosing the most room across ${inRotation.length} account${inRotation.length === 1 ? '' : 's'}. The next run uses ${name}.`;
}

/** Under the failover checkbox while chats are pinned: what it also does
 *  (`connectors::demoted_ids`). */
export const STEP_ASIDE_NOTE = "While Claude Code's login is out of room, a claude.ai connector set to Always comes along only when a message needs it, so other chats use your added accounts. The chat says when one was left out.";

/** The badge on the added account the routing picks: while chats are
 *  pinned to the login, only runs without a claude.ai connector go there. */
export function nextRunBadge(view: ClaudeAccountsView, pin: LoginPin | null): string {
  return pin?.connectors.length && !loginOutOfRoom(view) ? 'Next run without a claude.ai connector' : 'Next run';
}

/** The login row's badge: where chats go while a pin holds. */
export function loginRowBadge(view: ClaudeAccountsView): string {
  return loginOutOfRoom(view) ? 'Only messages that need its connectors' : 'Chats run here';
}

/** The login row's status from its windows as last seen: at 100% or more
 *  `limited`, at the ceiling `near`, else `ok`; null when never seen. */
export function loginStatus(view: ClaudeAccountsView): ClaudeAccountStatus | null {
  const u = view.loginUsage;
  if (!u) return null;
  const fullest = Math.max(u.fiveHour?.usedPercent ?? 0, u.sevenDay?.usedPercent ?? 0);
  if (fullest >= 100) return 'limited';
  return fullest >= view.ceilingPercent ? 'near' : 'ok';
}

/** Mirror of `claude_accounts::login_out_of_room`: the failover setting on,
 *  the login at or past its line (the Spread ceiling, 100% with one
 *  account) and an added account with room (an unmeasured one counts). */
export function loginOutOfRoom(view: ClaudeAccountsView): boolean {
  const status = loginStatus(view);
  const past = view.routing === 'spread' ? status === 'near' || status === 'limited' : status === 'limited';
  return view.failover && past && view.accounts.some(a => a.enabled && (a.status === 'ok' || a.status === 'unknown'));
}

/** The login row's explanation on Settings, Accounts while a pin holds. */
export function loginPinNote(pin: LoginPin, failover = true): string {
  const one = pin.connectors.length === 1;
  return `${joinNames(pin.connectors)} ${one ? 'is' : 'are'} set to Always, so ${one ? 'it rides' : 'they ride'} on every chat${failover ? ' while this login has room' : ''}, and a run that carries a claude.ai connector has to use this login. Such a run never moves to an added account, even past its limit. Set ${one ? 'it' : 'them'} to “Only when relevant”, or use Hollow’s own sign-in, to spread every chat.`;
}

/** Accounts a `single` routing can pin: every enabled one, plus the pinned one. */
export function pinnable(view: ClaudeAccountsView): ClaudeAccount[] {
  return view.accounts.filter(a => a.enabled || a.id === view.pinned);
}

/** Client-side mirror of the backend's token check, for instant feedback. */
export function tokenProblem(raw: string): string | null {
  const token = raw.replace(/\s+/g, '');
  if (!token) return null;
  if (token.startsWith('sk-ant-api')) return 'That is an API key (billed per token). Paste the sk-ant-oat01-… token from `claude setup-token`.';
  if (!token.startsWith('sk-ant-oat')) return 'Expected the sk-ant-oat01-… token that `claude setup-token` prints.';
  if (token.length < 40) return 'That token looks incomplete.';
  return null;
}

// R1's chat notice lives in accountSwitchNotice.ts, so the chat (App chunk)
// doesn't carry this module (P1-42).
export { accountSwitchNotice } from './accountSwitchNotice';

/** Explain policy without changing accounts, probing credentials or promising
 * eligibility from an unknown/stale meter. */
export function failoverSummary(view: ClaudeAccountsView): string {
  const scope = 'Automatic failover changes Claude accounts, not models or providers. It never selects a paid API.';
  const enabled = view.accounts.filter(a => a.enabled && a.status !== 'invalid' && a.status !== 'missingToken');
  if (enabled.length < 2) return `No second enabled, signed-in Claude account is currently reported. Check Accounts before expecting a switch. ${scope}`;
  if (enabled.every(a => a.status === 'limited')) return `All observed accounts report their limit; wait for a reset or check their usage. ${scope}`;
  return `Hollow continues on the eligible Claude account with the most room after a quota rejection, trying each account once per turn. When every account is out, it waits for the earliest reset. Availability depends on usage and organization policy. ${scope}`;
}
