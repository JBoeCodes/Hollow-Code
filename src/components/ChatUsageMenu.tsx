import { inSelectPopover } from './Select';
/**
 * The composer's usage popover: a ring showing how full the conversation's
 * context window is, opening a card with the context figure, the last turn's
 * tokens and cost, the Claude plan's 5-hour and weekly limits, and which
 * account runs this chat (for a self-hosted turn, its endpoint instead of the
 * account and plan, Q8).
 *
 * Plan limits come from the added Claude account new runs use (its own
 * readings, no Keychain), else Claude Code's login through `claude_quota`,
 * which is gated by the same per-machine opt-in as Settings, Usage's card,
 * because it may read the Keychain.
 */
import { quotaOptIn, setQuotaOptIn } from '../store/prefs';
import { For, Show, createEffect, createResource, createSignal, onCleanup } from 'solid-js';
import { api } from '../api';
import { createExitPhase } from './exit';
import {
  AGENT_LABELS, chatAgent, chatAgentSessionKey, chatModel, chatContextUsage, chatTurnUsage, currentAgentCapabilities, openInBrowser, openSettingsFromOutside,
} from '../store';
import {
  ANTIGRAVITY_CREDITS_URL, billingLabel, cacheRows, contextSummary, contextWindow, handoverOffered, messageContextSplit, planUsage,
  popoverEndpoint, selfHostedEndpoint, turnRows, type TurnRow,
} from '../chatUsage';
import { Icon } from '../icons';
import { openExternal } from '../utils';
import { modalOpen } from '../modalGuard';
import { formatTokens } from '../usageFormat';
import { costChip } from '../usageFormat';
import type { ClaudeAccountsView, ClaudeQuota, ProviderAccount } from '../types';
import '../styles/chat-usage-menu.css';
import { PlanWindows } from './PlanWindows';
import { selfHostedChatWindow } from '../store/selfHostedWake';


const RING = 2 * Math.PI * 7;

const Rows = (p: { rows: TurnRow[] }) => (
  <dl class="usage-menu-rows"><For each={p.rows}>{r => <><dt>{r.label}</dt><dd title={r.value}>{r.value}</dd></>}</For></dl>
);

/** `onContinue` (repo features R6): start a new conversation with a written
 *  handoff, offered past the hand-over point (`handoverOffered`). */
export function ChatUsageMenu(props: { onContinue?: () => void } = {}) {
  const [open, setOpen] = createSignal(false);
  const [reading] = createResource(
    () => open() && chatAgentSessionKey() ? { key: chatAgentSessionKey()!, usage: chatTurnUsage() } : null,
    async ({ key }) => {
      const [context, manifest] = await Promise.allSettled([api.conversationContextUse(key), api.runContextManifest(null, key)]);
      return { context: context.status === 'fulfilled' ? context.value : null, manifest: manifest.status === 'fulfilled' ? manifest.value?.manifest : null };
    },
  );
  const used = () => reading.loading ? null : reading()?.context;
  const split = () => reading.loading ? [] : messageContextSplit(reading()?.manifest);
  const handover = () => {
    const u = chatContextUsage();
    return !!u && handoverOffered(u.tokens, contextWindow(chatAgent(), chatModel() || u.model, u.tokens, u.window ?? selfHostedChatWindow()));
  };
  const [accounts, setAccounts] = createSignal<ClaudeAccountsView | null>(null);
  const [quota, setQuota] = createSignal<ClaudeQuota | null>(null);
  const optedIn = quotaOptIn;
  const [provider, setProvider] = createSignal<ProviderAccount | null>(null);
  const phase = createExitPhase(open, 120);
  // Keeps the card inside the composer when the ring sits near its left edge
  // (narrow windows): its width, and how far past the ring its right edge moves.
  const [place, setPlace] = createSignal({ width: 340, shift: 0, height: 560 });
  let root: HTMLDivElement | undefined;
  let trigger: HTMLButtonElement | undefined;
  let lastSession = chatAgentSessionKey();
  createEffect(() => { const key = chatAgentSessionKey(); if (key !== lastSession) setOpen(false); lastSession = key; });

  const context = () => {
    const used = chatContextUsage();
    if (!used) return null;
    // A self-hosted chat's window is its Settings entry's (Q6: 131,072 for
    // Qwen), not the 200K the ring would infer for a Claude model.
    return contextSummary(used.tokens, contextWindow(chatAgent(), chatModel() || used.model, used.tokens, used.window ?? selfHostedChatWindow()));
  };
  const plan = () => planUsage(accounts(), quota());

  createEffect(() => {
    setProvider(null);
    setAccounts(null);
    if (!open()) return;
    if (root) {
      // The page clips overflow, so stay inside the composer's own box.
      const right = root.getBoundingClientRect().right;
      const box = (root.closest('.chat-composer') ?? document.body).getBoundingClientRect();
      const width = Math.min(340, box.width);
      setPlace({ width, shift: Math.max(0, box.left - (right - width)), height: Math.max(100, root.getBoundingClientRect().top - Math.max(0, root.closest('.chat-main')?.getBoundingClientRect().top ?? 0) - 16) });
    }
    const agent = chatAgent();
    let current = true;
    onCleanup(() => { current = false; });
    if (agent === 'claude') {
      void api.claudeAccounts().then(value => { if (current) setAccounts(value); }, () => {});
    } else if (agent === 'codex' || agent === 'grok') {
      void api.providerAccounts().then(list => {
        if (current) setProvider(list.find(p => p.id === agent) ?? null);
      }, () => {});
    }
    const onDown = (e: PointerEvent) => { if (root && !root.contains(e.target as Node) && !inSelectPopover(e.target)) setOpen(false); };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.isComposing || modalOpen() || document.querySelector('.keymap-backdrop')) return;
      e.preventDefault(); e.stopPropagation();
      if (root?.contains(document.activeElement)) trigger?.focus();
      setOpen(false);
    };
    const resize = () => setOpen(false);
    window.addEventListener('resize', resize);
    // A sidebar or workspace panel toggle resizes the pane without a window
    // resize, which would leave the card placed for the old width: close it
    // the same way (ENG-93). Only a width change counts: the first callback
    // is the observer's own start and records the width, and a height change
    // (the composer growing, a banner) leaves the card where it is.
    const pane = root?.closest('.chat-main') ?? null;
    let watching: ResizeObserver | undefined;
    if (pane && typeof ResizeObserver !== 'undefined') {
      let width: number | undefined;
      watching = new ResizeObserver(entries => {
        const now = entries[entries.length - 1]?.contentRect.width;
        if (now === undefined) return;
        if (width === undefined) width = now;
        else if (now !== width) setOpen(false);
      });
      watching.observe(pane);
    }
    document.addEventListener('pointerdown', onDown, true);
    document.addEventListener('keydown', onKey, true);
    onCleanup(() => {
      document.removeEventListener('pointerdown', onDown, true);
      document.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', resize);
      watching?.disconnect();
    });
  });

  // Opting into plan limits must not reset the open context/cache disclosure
  // or start another account read. This request has its own provider owner.
  createEffect(() => {
    setQuota(null);
    if (!open() || chatAgent() !== 'claude' || !optedIn()) return;
    let current = true;
    onCleanup(() => { current = false; });
    void api.claudeQuota(false).then(value => { if (current) setQuota(value); }, () => {});
  });

  function optIn() {
    setQuotaOptIn(true);
  }

  const fraction = () => context()?.fraction ?? null;
  const level = () => {
    const f = fraction();
    return f == null ? 'none' : f >= 0.9 ? 'over' : f >= 0.75 ? 'high' : 'ok';
  };
  // Q8: a self-hosted turn ran on its endpoint, never a Claude account or plan.
  // A running turn clears its usage, so the conversation's last endpoint stays
  // until the next usage event (`popoverEndpoint`).
  const [remembered, setRemembered] = createSignal<{ key: string | null; endpoint: string | null }>({ key: null, endpoint: null });
  createEffect(() => {
    const usage = chatTurnUsage();
    if (usage) setRemembered({ key: chatAgentSessionKey(), endpoint: selfHostedEndpoint(usage) });
  });
  const endpoint = () => {
    const r = remembered();
    return popoverEndpoint(chatTurnUsage(), chatModel(), r.key === chatAgentSessionKey() ? r.endpoint : null);
  };
  const account = () => {
    if (endpoint()) return null;
    if (chatAgent() === 'claude') return plan()?.account ?? (accounts()?.nextAccount ? null : 'Claude Code login');
    return provider()?.id === chatAgent() ? provider()?.email ?? null : null;
  };
  const chip = () => costChip(chatTurnUsage(), chatAgent());
  const reportsContext = () => currentAgentCapabilities()?.reportsContext ?? true;

  return (
    <div class="usage-menu" ref={root}>
      <button
        type="button"
        ref={trigger}
        class="usage-menu-trigger"
        data-level={level()}
        aria-label={context() ? `Context ${context()!.label}. Show usage` : 'Show usage'}
        aria-expanded={open()}
        title={[context() ? `Context ${context()!.label}` : 'Usage', chip() ? `Last turn ${chip()!.label}` : ''].filter(Boolean).join(' · ')}
        onClick={() => setOpen(o => !o)}
      >
        <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
          <circle class="usage-ring-track" cx="9" cy="9" r="7" />
          <circle
            class="usage-ring-fill"
            cx="9" cy="9" r="7"
            stroke-dasharray={`${RING}`}
            stroke-dashoffset={`${RING * (1 - (fraction() ?? 0))}`}
          />
        </svg>
      </button>
      <Show when={phase() !== 'closed'}>
        <div
          class={`usage-menu-pop ${phase() === 'closing' ? 'is-closing' : ''}`}
          role="dialog"
          aria-label="Usage"
          style={{ width: `${place().width}px`, right: `${-place().shift}px`, "max-height": `${place().height}px` }}
        >
          <section class="usage-menu-section">
            <div class="usage-menu-head">
              <h3>Context</h3>
              <Show when={context()}>{c => <span class="usage-menu-figure">{c().label}</span>}</Show>
            </div>
            <Show
              when={context()}
              fallback={<p class="usage-menu-note">{reportsContext()
                ? 'Shown after the next turn finishes.'
                : `${AGENT_LABELS[chatAgent()]} doesn't report how full its context is.`}</p>}
            >
              {c => (
                <Show when={c().fraction != null}>
                  <div class="usage-menu-bar" data-level={level()}><span style={{ width: `${(c().fraction ?? 0) * 100}%` }} /></div>
                </Show>
              )}
            </Show>
            <Show when={turnRows(chatTurnUsage()).length > 0 || chip()}>
              <h4 class="usage-menu-subhead">Last turn</h4>
              <dl class="usage-menu-rows">
                <For each={turnRows(chatTurnUsage())}>
                  {row => <><dt>{row.label}</dt><dd>{row.value}</dd></>}
                </For>
                <Show when={chip()}>{c => <><dt>Cost</dt><dd title={c().title}>{c().label}</dd></>}</Show>
              </dl>
            </Show>
            <Show when={chatAgentSessionKey()}>
              <details class="usage-menu-more">
                <summary><Icon name="chevronRight" size={12} />What's using it</summary>
                <p class="usage-menu-note">Latest message · recorded text</p>
                <Show when={split().length} fallback={<p class="usage-menu-note">{reading.loading ? 'Reading…' : 'Breakdown unavailable'}</p>}>
                  <p class="usage-menu-split"><For each={split()}>{(r, i) => <><Show when={i() > 0}> · </Show><span>{r.label} {r.value}</span></>}</For></p>
                </Show>
                <button type="button" class="usage-menu-link" onClick={() => { setOpen(false); openSettingsFromOutside('context'); }}>Context details</button>
              </details>
              <details class="usage-menu-more">
                <summary><Icon name="chevronRight" size={12} />Cache</summary>
                <Show when={chatTurnUsage()?.cachedInputTokens != null}>
                  <Rows rows={[{ label: 'Last turn · cache reads', value: formatTokens(chatTurnUsage()!.cachedInputTokens!) }]} />
                </Show>
                <p class="usage-menu-note">Repeated reads across the turn's model calls. This total can exceed the context window.</p>
                <Show when={used()} fallback={<p class="usage-menu-note">{reading.loading ? 'Reading…' : 'Cache history unavailable'}</p>}>
                  {u => <>
                    <Rows rows={cacheRows(u())} />
                    <p class="usage-menu-note">Compactions this agent session: {u().compactions}</p>
                  </>}
                </Show>
              </details>
            </Show>
            <Show when={props.onContinue && handover()}>
              <button
                type="button"
                class="btn usage-menu-optin"
                title="Start a new conversation with a written handoff of this one"
                onClick={() => { setOpen(false); props.onContinue?.(); }}
              >
                Continue in a new conversation
              </button>
            </Show>
          </section>

          <Show when={chatAgent() === 'claude' && !endpoint()}>
            <section class="usage-menu-section">
              <div class="usage-menu-head"><h3>Claude usage</h3></div>
              <Show
                when={plan()}
                fallback={
                  <Show
                    when={!optedIn() && !accounts()?.nextAccount}
                    fallback={<p class="usage-menu-note">Plan limits appear after Hollow's next Claude turn.</p>}
                  >
                    <p class="usage-menu-note">
                      Read your plan's limits from Claude Code's login (macOS may ask for Keychain access).
                    </p>
                    <button type="button" class="btn usage-menu-optin" onClick={optIn}>Show plan usage</button>
                  </Show>
                }
              >
                {p => (
                  <Show when={p().windows.length > 0} fallback={<p class="usage-menu-note">Not measured yet. Check usage in Settings → Usage → Limits.</p>}>
                    <PlanWindows windows={p().windows} />
                  </Show>
                )}
              </Show>
            </section>
          </Show>

          <Show when={chatAgent() === 'codex'}>
            <section class="usage-menu-section">
              <div class="usage-menu-head"><h3>Codex usage</h3></div>
              <Show when={provider()?.limits?.windows.length}
                fallback={<p class="usage-menu-note">Codex didn't report limits.</p>}>
                <PlanWindows windows={provider()!.limits!.windows} />
                <p class="usage-menu-note">Last reported {new Date(provider()!.limits!.observedAt).toLocaleString()}.</p>
              </Show>
            </section>
          </Show>

          <section class="usage-menu-section">
            <dl class="usage-menu-rows">
              <dt>Agent</dt><dd>{AGENT_LABELS[chatAgent()]}</dd>
              <Show when={endpoint()}>{e => <><dt>Endpoint</dt><dd>{e()}</dd></>}</Show>
              <Show when={account()}>{a => <><dt>Account</dt><dd>{a()}</dd></>}</Show>
              <Show when={!endpoint() && provider()?.id === chatAgent() && provider()?.plan}><dt>Plan</dt><dd>{provider()!.plan}</dd></Show>
              <Show when={billingLabel(chatTurnUsage()?.billing)}>{b => <><dt>Billing</dt><dd>{b()}</dd></>}</Show>
              <Show when={chatAgent() === 'antigravity'}>
                <dt>Credits</dt>
                <dd><button type="button" class="usage-menu-link" title="Antigravity doesn't report the prepaid credit balance; AI Studio shows it"
                  onClick={() => void openInBrowser(ANTIGRAVITY_CREDITS_URL).catch(() => openExternal(ANTIGRAVITY_CREDITS_URL))}>Balance in AI Studio</button></dd>
              </Show>
            </dl>
            <button
              type="button"
              class="usage-menu-link"
              onClick={() => { setOpen(false); openSettingsFromOutside('usage'); }}
            >
              Usage details
            </button>
          </section>
        </div>
      </Show>
    </div>
  );
}
