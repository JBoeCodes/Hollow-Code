// Plaud connector health: status, connect and disconnect.
// Moved verbatim out of store/runtime.ts (plan L3 X19, 2026-10-14); runtime.ts
// re-exports every symbol, so `store/runtime` imports keep working.
import { createSignal } from 'solid-js';
import { api } from '../api';
import { errMsg } from '../utils';
import { trackTask } from './activityDock';

// ─────────────────────────────────────────────────────────────
// PLAUD RECORDINGS — Hollow's own Plaud sign-in for Library → Recordings
// (Recordings in Hollow, P1-20): read on demand, nothing copied unless the
// person keeps a transcript. Auth is the `plaud` CLI's own OAuth (`plaud
// login`), tokens at ~/.plaud/tokens.json; agents use the Plaud connector.
// ─────────────────────────────────────────────────────────────

export type PlaudStatus = 'unknown' | 'ok' | 'needs-auth' | 'authenticating';
export const [plaudStatus, setPlaudStatus] = createSignal<PlaudStatus>('unknown');
export const [plaudMessage, setPlaudMessage] = createSignal<string>('');

/// Does an error look like the `plaud` CLI needs `plaud login`? Flips the
/// connector to needs-auth (mirrors looksLikeM365AuthError).
export function looksLikePlaudAuthError(msg: string | null | undefined): boolean {
  if (!msg) return false;
  const l = msg.toLowerCase();
  return l.includes('not authenticated')
      || l.includes('plaud login')
      || l.includes('auth_failed')
      || l.includes('token invalid')
      || l.includes('token expired');
}

/// Cheap boot check — are the CLI's OAuth tokens present? (no network call)
export async function checkPlaudStatus(): Promise<void> {
  if (plaudStatus() === 'authenticating') return;
  try {
    const connected = await api.plaudConnected();
    setPlaudStatus(connected ? 'ok' : 'needs-auth');
    setPlaudMessage(connected ? 'Plaud connected.' : 'Plaud not connected.');
  } catch (err) {
    setPlaudStatus('unknown');
    setPlaudMessage(errMsg(err));
  }
}

/// Run `plaud login` (opens the browser, blocks on the OAuth callback).
/// Recordings stay in Plaud (agents read them through the Plaud connector);
/// Hollow no longer pulls them into the vault.
export async function connectPlaud(): Promise<void> {
  if (plaudStatus() === 'authenticating') return;
  setPlaudStatus('authenticating');
  setPlaudMessage('Opening Plaud sign-in in your browser…');
  try {
    await trackTask('Connect Plaud', () => api.plaudLogin(), {
      detail: 'OAuth flow with browser callback',
      onSuccessResult: () => 'signed in',
      onSuccessStatus: () => 'success',
    });
    setPlaudStatus('ok');
    setPlaudMessage('Plaud connected.');
  } catch (err) {
    const msg = errMsg(err);
    setPlaudStatus(looksLikePlaudAuthError(msg) ? 'needs-auth' : 'unknown');
    setPlaudMessage(msg);
  }
}

/// Sign out of Plaud (revokes the CLI's stored tokens via `plaud logout`).
export async function disconnectPlaud(): Promise<void> {
  try {
    await api.plaudLogout();
  } catch { /* best-effort — clear local status regardless */ }
  setPlaudStatus('needs-auth');
  setPlaudMessage('Plaud disconnected.');
}
