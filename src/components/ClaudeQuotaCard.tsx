/**
 * Settings, Usage "Claude plan" card (Paperclip PR 8c, gate D2b).
 *
 * Display only: the five-hour and weekly windows of the user's Claude plan,
 * from the `claude_quota` command. Nothing is invoked until the user opts in
 * (the opt-in is this machine's localStorage, never the vault), because the
 * backend may read Claude Code's login from the Keychain to ask Claude's usage
 * service. A null answer renders nothing. Only a Keychain or endpoint failure
 * is sticky: the backend then keeps the card hidden until Hollow restarts, so
 * it can never prompt twice. A null with nothing observed yet (an account
 * with no subscription evidence, or no Hollow Claude run since launch) is not:
 * the card appears on a later visit once one of Hollow's own Claude runs
 * reports its limits.
 *
 * Refresh runs through trackTask, since a Keychain read plus the request can
 * take a few seconds; a failure still hides the card rather than showing an
 * error for display-only data.
 *
 * A manual signal, not a resource: the card sits inside the Settings Suspense
 * boundary, and a resource read would blank the whole Usage section on every
 * Refresh.
 */
import { quotaOptIn, setQuotaOptIn } from '../store/prefs';
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import { trackTask } from '../store';
import { percentUsed, planLevel } from '../chatUsage';
import type { ClaudeQuota, ClaudeQuotaWindow } from '../types';
import '../styles/claude-quota.css';


/** The meter fill only: clamped to the track. */
function fillFraction(n: number): number {
  return Math.min(Math.max(n / 100, 0), 1);
}

function resetLabel(iso: string | null): string | null {
  if (!iso) return null;
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return null;
  return `Resets ${at.toLocaleString([], { weekday: 'short', hour: 'numeric', minute: '2-digit' })}`;
}

function ageLabel(iso: string): string {
  const at = new Date(iso).getTime();
  if (Number.isNaN(at)) return '';
  const minutes = Math.round((Date.now() - at) / 60_000);
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(at).toLocaleDateString();
}

function sourceLabel(w: ClaudeQuotaWindow): string {
  return w.source === 'stream' ? 'from your last Claude run' : "from Claude's usage service";
}

export function ClaudeQuotaCard() {
  const optedIn = quotaOptIn;
  // undefined: not asked yet; null: hidden for this session.
  const [quota, setQuota] = createSignal<ClaudeQuota | null | undefined>(undefined);
  const [busy, setBusy] = createSignal(false);

  async function load(force: boolean) {
    if (busy()) return;
    setBusy(true);
    try {
      const next = force
        ? await trackTask('Refresh Claude plan usage', () => api.claudeQuota(true), {
            onSuccessResult: q => (q ? undefined : 'Not available; the card is hidden'),
            onSuccessStatus: q => (q ? 'success' : 'warning'),
          })
        : await api.claudeQuota(false);
      setQuota(next);
    } catch {
      // The command never rejects on a read failure; anything else still
      // hides the card rather than showing an error for display-only data.
      setQuota(null);
    } finally {
      setBusy(false);
    }
  }

  onMount(() => {
    if (optedIn()) void load(false);
  });

  function optIn() {
    setQuotaOptIn(true);
    void load(false);
  }

  return (
    <>
      <Show when={!optedIn()}>
        <div class="claude-quota-optin">
          <div class="claude-quota-optin-text">
            <strong>Claude plan usage</strong>
            <span>
              Your plan's 5-hour and weekly limits, read from Hollow's own Claude runs and, when
              you ask, from Claude's usage service with your Claude Code login (macOS may ask for
              Keychain access). If it can't be read, the card stays hidden until Hollow restarts.
            </span>
          </div>
          <button class="btn" onClick={optIn}>Show Claude plan usage</button>
        </div>
      </Show>

      <Show when={optedIn() && busy() && quota() === undefined}>
        <p class="claude-quota-note">Checking Claude plan usage…</p>
      </Show>

      <Show when={optedIn() ? quota() : null}>
        {q => (
          <section class="claude-quota-card" aria-label="Claude plan usage">
            <header class="claude-quota-head">
              <h3 class="claude-quota-title">Claude plan</h3>
              <span class="claude-quota-age" title={q().observedAt}>Updated {ageLabel(q().observedAt)}</span>
              <button
                class="btn claude-quota-refresh"
                onClick={() => void load(true)}
                disabled={busy()}
                title="Update the card. With a Claude subscription login this may ask Claude's usage service (at most once a minute); otherwise it shows what Hollow's own Claude runs last reported. If the login or the service can't be read, the card hides until Hollow restarts."
              >
                Refresh
              </button>
            </header>
            <ul class="claude-quota-windows">
              <For each={q().windows}>
                {w => (
                  <li class="claude-quota-window" data-level={planLevel(w.usedPercent)} data-window={w.id}>
                    <div class="claude-quota-row">
                      <span class="claude-quota-label">{w.label}</span>
                      <span class="claude-quota-percent">{percentUsed(w.usedPercent)}</span>
                    </div>
                    <div class="meter claude-quota-meter" aria-hidden="true">
                      <span class="meter-track">
                        <span class="meter-fill" style={{ '--pct': fillFraction(w.usedPercent) }} />
                      </span>
                    </div>
                    <div class="claude-quota-meta">
                      <Show when={resetLabel(w.resetsAt)}>{label => <span>{label()}</span>}</Show>
                      <span title={w.observedAt}>{sourceLabel(w)}</span>
                    </div>
                  </li>
                )}
              </For>
            </ul>
            <p class="claude-quota-note">
              Display only: Hollow never limits runs by this. Over 100% means the plan is using
              overage.
            </p>
          </section>
        )}
      </Show>
    </>
  );
}
