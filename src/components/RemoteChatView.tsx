// M2 PR 4: a chat on another Mac, read live (`docs/reference/peers.md`,
// "The UI"). Opened from the machine strip into the page pane, over whatever
// page is there, and never through chatRuntime.ts: local chats keep their own
// path untouched. Its turns live in `store/peerChat.ts` while the view is open;
// closing it drops them and stops the stream.
//
// What crosses the link is live control only, so the view is read-only apart
// from a text composer (Send, Stop) and the chat's open card, answered at its
// home with the existing QuestionCard. Canvas fences show a placeholder card
// (canvas routes are denied to another Mac); attachments show by name.
import { For, Show, createEffect, createMemo, createSignal, on, onCleanup, onMount } from 'solid-js';
import type { InteractionAnswer } from '../types';
import { parseSegments } from '../chatSegments';
import { PERM_MODE_SHORT_LABELS, type ChatPermissionMode } from '../store/chatRuntime';
import { flashFooter } from '../store';
import { errMsg, openExternal } from '../utils';
import { Icon } from '../icons';
import { Button, ErrorText, IconButton } from './ui';
import { CodeBlock } from './CodeBlock';
import { MarkdownView } from './MarkdownView';
import { QuestionCard } from './QuestionCard';
import { MachineBadge } from './MachineBadge';
import { peerMachines } from '../store/peerSessions';
import {
  answerRemote, closeRemoteChat, loadEarlierRemoteTurns, openCardOf, openRemoteChat, remoteChat, sendRemote, stopRemote,
  wireQuestionsToCard, type WireTurn,
} from '../store/peerChat';
import '../styles/peer-machines.css';

/** A remote reply's text: prose, code, and a placeholder for each canvas. */
function RemoteText(props: { text: string; home: string }) {
  const segments = createMemo(() => parseSegments(props.text));
  // Links in a remote reply: a web link opens in the browser; a file path
  // names a file on the other Mac, which this Mac can't open.
  const link = (e: MouseEvent | KeyboardEvent) => {
    const a = (e.target as Element | null)?.closest?.('a');
    if (!a) return;
    e.preventDefault();
    const href = a.getAttribute('href') ?? '';
    if (/^https?:\/\//i.test(href)) openExternal(href);
    else flashFooter(`That link is a file on ${props.home}. Open it there.`, 3500);
  };
  return (
    <For each={segments()}>
      {seg => seg.kind === 'md'
        ? <MarkdownView text={seg.text} class="turn-md assistant" forbidMedia onLinkClick={link} />
        : seg.kind === 'canvas'
          ? <div class="remote-canvas-card" role="note"><Icon name="presentation" size={15} /><span>Canvas on {props.home}: open it there.</span></div>
          : <CodeBlock code={seg.code} lang={seg.lang} />}
    </For>
  );
}

const toolNames = (tools: readonly unknown[]): string[] => tools
  .map(t => (typeof t === 'object' && t !== null && typeof (t as { name?: unknown }).name === 'string' ? (t as { name: string }).name : ''))
  .filter(Boolean);

function TurnTools(props: { names: string[] }) {
  return (
    <Show when={props.names.length > 0}>
      <p class="remote-turn-tools">{props.names.length === 1 ? 'Used ' : `${props.names.length} tools: `}{[...new Set(props.names)].join(', ')}</p>
    </Show>
  );
}

function Turn(props: { turn: WireTurn; home: string }) {
  const t = () => props.turn;
  return (
    <div class={`turn ${t().role} remote-turn`} data-ord={t().ord}>
      <Show
        when={t().role === 'assistant'}
        fallback={
          <div class="chat-user-chip">
            <span class="sr-only">Prompt</span>
            <Show when={t().attachments.length > 0}>
              <ul class="remote-turn-files" aria-label="Attached on the other Mac">
                <For each={t().attachments}>{a => <li><Icon name="fileText" size={13} />{a.name}</li>}</For>
              </ul>
            </Show>
            <Show when={t().text}><div class="text">{t().text}</div></Show>
          </div>
        }
      >
        <div class="turn-content">
          <TurnTools names={t().tools.map(x => x.name)} />
          <RemoteText text={t().text} home={props.home} />
          <Show when={t().truncated}><p class="remote-turn-note">Cut short here. The whole reply is on {props.home}.</p></Show>
          <Show when={t().error}>{e => <ErrorText text={e()} class="remote-turn-note" />}</Show>
        </div>
      </Show>
    </div>
  );
}

/** What a failed send, stop or answer says. */
function failureText(e: unknown, home: string): string {
  const f = e as { code?: string; message?: string } | null;
  if (f?.code === 'busy') return `${home} is busy with this chat. Your words are still here; send again in a moment.`;
  if (f?.code === 'question-closed') return `This card was answered or replaced on ${home}.`;
  return f?.message || errMsg(e);
}

export default function RemoteChatView() {
  const chat = remoteChat;
  const home = () => chat()?.home?.name ?? peerMachines()?.machines.find(m => m.machineId === chat()?.machineId)?.name ?? 'the other Mac';
  const homeRef = () => chat()?.home ?? (() => {
    const m = peerMachines()?.machines.find(x => x.machineId === chat()?.machineId);
    return m ? { machineId: m.machineId, slug: m.slug, name: m.name } : null;
  })();
  const title = () => chat()?.session?.title?.trim() || 'Chat';
  const mode = () => {
    const m = chat()?.permissionMode;
    return m ? PERM_MODE_SHORT_LABELS[m as ChatPermissionMode] ?? m : null;
  };
  const running = () => !!chat()?.live || ['running', 'queued'].includes(chat()?.session?.state ?? '');
  /** Why Send and Stop can't be pressed now, or null. */
  const blocked = () => {
    const c = chat();
    if (!c) return 'No chat is open.';
    if (c.ended) return c.ended.message;
    if (c.offline) return `Can't reach ${home()}. ${c.offline}`;
    return null;
  };

  const [draft, setDraft] = createSignal('');
  const [sending, setSending] = createSignal(false);
  const [note, setNote] = createSignal('');
  const [answering, setAnswering] = createSignal(false);
  const [cardError, setCardError] = createSignal<string | null>(null);
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  let scroller: HTMLDivElement | undefined;
  let box: HTMLTextAreaElement | undefined;

  // One object per open card (its id and state), so the card keeps what the
  // person picked while the chat around it streams; keyed below, so nothing
  // reads a card that has closed.
  const card = createMemo(() => {
    const c = chat();
    const open = c ? openCardOf(c.turns) : null;
    if (!c || !open || open.questions.id === dismissed()) return null;
    return { questions: open.questions, card: wireQuestionsToCard(c.key, open.turn, open.questions, home()) };
  }, null, { equals: (a, b) => a?.questions.id === b?.questions.id && a?.questions.state === b?.questions.state && a?.card.sessionKey === b?.card.sessionKey });

  // Follow the newest words while the reader is at the bottom.
  let pinned = true;
  const atBottom = () => !scroller || scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  createEffect(on(() => [chat()?.turns.length, chat()?.live?.text.length, !!card()], () => {
    if (pinned && scroller) queueMicrotask(() => { if (scroller) scroller.scrollTop = scroller.scrollHeight; });
  }));
  // A new chat starts at its end with an empty box.
  createEffect(on(() => chat() && `${chat()!.machineId}:${chat()!.key}`, () => { pinned = true; setDraft(''); setNote(''); setCardError(null); }, { defer: true }));

  onMount(() => {
    box?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || document.querySelector('.ctx-menu, .machine-popover, [role="dialog"][aria-modal="true"]')) return;
      // Focus in the chat, on its layer, or nowhere (the page under it is inert).
      const at = document.activeElement;
      if (at && at !== document.body && !at.closest('.remote-chat-layer')) return;
      // A first Escape in a box with words clears them; the next one closes.
      if (at === box && draft()) { e.preventDefault(); setDraft(''); setNote('Draft cleared. Escape again closes the chat.'); return; }
      close();
    };
    document.addEventListener('keydown', key);
    onCleanup(() => document.removeEventListener('keydown', key));
  });

  function close() {
    closeRemoteChat();
  }

  async function send() {
    const text = draft().trim();
    if (!text || sending() || blocked()) return;
    setSending(true); setNote('');
    try {
      await sendRemote(text);
      setDraft('');
      pinned = true;
    } catch (e) {
      setNote(failureText(e, home()));
    } finally {
      setSending(false);
    }
  }

  async function stop() {
    setNote('');
    try {
      await stopRemote();
    } catch (e) {
      setNote(failureText(e, home()));
    }
  }

  // An answer sent holds the card until the home says it closed (a new card
  // or none), so a second click can't send it twice.
  createEffect(on(card, () => setAnswering(false), { defer: true }));
  // If the card never closes here (its event and the refetch both lost), the
  // card opens again after a while rather than staying locked.
  let unlock: number | undefined;
  onCleanup(() => window.clearTimeout(unlock));
  async function answer(questions: NonNullable<ReturnType<typeof card>>['questions'], value: InteractionAnswer) {
    if (answering()) return;
    setAnswering(true); setCardError(null);
    try {
      await answerRemote(questions, value);
      window.clearTimeout(unlock);
      unlock = window.setTimeout(() => setAnswering(false), 10_000);
    } catch (e) {
      setCardError(failureText(e, home()));
      setAnswering(false);
    }
  }

  async function copyId() {
    const key = chat()?.key;
    if (!key) return;
    try { await navigator.clipboard.writeText(key); flashFooter('Conversation ID copied'); } catch { flashFooter('× Could not copy', 2500); }
  }

  async function retry() {
    const c = chat();
    if (c) await openRemoteChat(c.machineId, c.key);
  }

  return (
    <Show when={chat()}>
      {c => (
        <section class="remote-chat" role="region" aria-label={`Chat on ${home()}`}>
          <header class="remote-chat-head">
            <div class="remote-chat-heading">
              <h2 class="remote-chat-title" title={title()}>{title()}</h2>
              <Show when={homeRef()}>{ref => <MachineBadge machine={ref()} needsAnswer={!!c().session?.needsAnswer} />}</Show>
            </div>
            <p class="remote-chat-sub">
              <span class="live-dot" classList={{ on: c().connected && !c().offline }} aria-hidden="true" />
              <span>Live from {home()}</span>
              <Show when={mode()}>{m => <span class="remote-chat-mode" title={`Set on ${home()}; read-only here`}>Mode: {m()}</span>}</Show>
            </p>
            <span class="remote-chat-tools">
              <IconButton icon="copy" size={14} label="Copy conversation ID" onClick={() => void copyId()} />
              <IconButton icon="x" size={15} label="Close this chat" onClick={close} />
            </span>
          </header>
          <Show when={c().ended ?? (c().offline ? { message: c().offline! } : null)}>
            {why => (
              <div class="remote-chat-banner" role="status">
                <Icon name="wifiOff" size={14} />
                <span>{c().ended ? why().message : `Can't reach ${home()}. ${why().message}`}</span>
                <Show when={!c().ended}><Button variant="ghost" size="sm" onClick={() => void retry()}>Retry</Button></Show>
              </div>
            )}
          </Show>
          <div class="remote-chat-scroll" ref={el => (scroller = el)} tabindex="-1" onScroll={() => { pinned = atBottom(); }}>
            <div class="remote-chat-turns">
              <Show when={c().hasMore}>
                <button type="button" class="chat-show-earlier" onClick={() => void loadEarlierRemoteTurns()}><Icon name="history" size={13} />Load earlier</button>
              </Show>
              <Show when={c().turns.length === 0 && !c().live}>
                <p class="remote-chat-empty" role="status">{c().session ? 'No messages yet.' : `Opening the chat on ${home()}…`}</p>
              </Show>
              <For each={c().turns}>{t => <Turn turn={t} home={home()} />}</For>
              <Show when={c().live}>
                {live => (
                  <div class="turn assistant partial remote-turn" aria-live="polite" aria-busy="true">
                    <div class="turn-content">
                      <TurnTools names={toolNames(live().tools)} />
                      <Show when={live().text} fallback={<p class="remote-turn-note">Working on {home()}…</p>}>
                        <RemoteText text={live().text} home={home()} />
                      </Show>
                    </div>
                  </div>
                )}
              </Show>
              <Show when={card()} keyed>
                {open => (
                  <div class="remote-chat-card">
                    <QuestionCard card={open.card} blockReason={open.card.status === 'pending' ? blocked() : open.card.statusReason}
                      submitting={answering()} error={cardError()}
                      onSubmit={value => void answer(open.questions, value)} onDismiss={() => setDismissed(open.questions.id)} />
                  </div>
                )}
              </Show>
            </div>
          </div>
          <form class="remote-chat-composer" aria-label={`Message ${home()}`} onSubmit={e => { e.preventDefault(); void send(); }}>
            <textarea
              ref={el => (box = el)}
              aria-label="Message"
              placeholder={blocked() ?? `Message this chat on ${home()}`}
              rows={2}
              value={draft()}
              disabled={!!c().ended}
              onInput={e => setDraft(e.currentTarget.value)}
              onKeyDown={e => {
                if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); void send(); }
              }}
            />
            <div class="remote-chat-actions">
              <p class="remote-chat-note" role="status">{note() || (blocked() && !c().ended ? blocked() : '')}</p>
              <Show when={running()}>
                <Button variant="ghost" size="sm" icon="stop" class="remote-chat-stop" disabled={!!blocked()} onClick={() => void stop()} aria-label={`Stop on ${home()}`}>Stop</Button>
              </Show>
              <button type="submit" class="btn primary remote-chat-send" aria-label="Send" title="Send (Enter)" disabled={!!blocked() || sending() || !draft().trim()}><Icon name="arrowUp" size={16} /></button>
            </div>
          </form>
        </section>
      )}
    </Show>
  );
}
