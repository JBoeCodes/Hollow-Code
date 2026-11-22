// The composer's send controller. Extracted from ChatBody.tsx (plan L3 X17,
// 2026-10-14; review V06/F11): the working-folder preflight, send and the
// queued dispatch every turn goes through (Paperclip PR 7b), Continue
// (PR 14b), question cards (PR 13), Stop and the Queued badge, the first
// message Your work leaves for a new quick chat, and the live activity line.
// Moved unchanged, no behaviour change.
//
// Created once, synchronously, in ChatBody's body (it owns the effects it
// sets up), with the component's props object so every read stays as lazy
// as it was. The composer's suggestion and slash-command handling reach it
// through `deps`.
import { batch, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import {
  chatTurns,
  chatSessionId,
  chatAgentSessionKey, chatContinuedFrom,
  chatCwd,
  chatBusy, setChatBusy,
  chatInput, setChatInput,
  chatModel,
  chatAgent, AGENT_LABELS, agentSettingsError, agentDescriptors, refreshAgentSettings,
  chatAttachments as attachments, setChatAttachments as setAttachments,
  chatSessionLoading,
  chatPermissionMode, chatUltracode, chatEffort, chatCanvases,
  pushChatTurn,
  recoverChatContext,
  chatQueuedTurn, isVisiblyQueued, isPreparingTurn, turnIsQueuedTurn, stopChatRun,
  beginQueuedChatTurn, acceptQueuedChatTurn, abandonQueuedChatTurn, removeChatUserTurn,
  setChatHistoryCursor, pushChatHistory,
  pendingChatSend, setPendingChatSend,
  flashFooter,
  pageContext, chatContextEnabled,
  agentsAvailable,
  refreshChatInteractions, removeChatInteraction, returnUnsentAnswers,
  chatActivity,
  ensureChatSteerSupport, steersNatively, steeringAgent, chatSteerSupport, chatTurnSteerSupport, addHeldChip, applyOwnSteer, adoptBusySend, takeEarlyHeld, registerBusySend, forgetBusySend, returnToConversation,
  acceptBackgroundSend, endBackgroundSend, returnUnsentAnswersTo,
  holdConversationForSend, withdrawStoppedSend,
  type ChatTurn,
  type ChatToolUse,
  type ChatAttachment,
} from '../../store';
import { api } from '../../api';
import { unavailableAgentNotice } from '../../store/agentHealth';
import { errMsg } from '../../utils';
import type { CannedRequest, ChatInteraction, InteractionAnswer, SendAccepted } from '../../types';
import { CONTINUE_PROMPT, canContinueChat } from '../../chatContinue';
import type { ShipAction } from '../../shipActions';
import { beginPreparingSend, CHAT_CONTEXT_RECOVERED_PREFIX, endPreparingSend, newClientTurnId, preparingSendIn, queuedFailureNotice, restoredDraft, type PreparingSend } from '../../chatQueue';
import { unsafeCwdNotice, unsafeCwdText } from '../../chatCwd';
import { progressLabel, quietLabel } from '../../runActivity';
import { attachmentImages, attachmentSendable, sharedPageBlock } from '../../sharedPage';
import { attachedFilesBlock, turnFilesField } from '../../composerFiles';
import { defaultBusyChoice, newSteerId, steerBlockedWhy, steerRefusal, steerRefusalNotice, steerTooLong, type BusyChoice } from '../../chatSteer';
import { busyCommandRefusal, busyLocalCommand } from './slashCommands';
import { dictationSent } from '../../store/dictation';
import type { ComposerField } from '../../composerField';

/** Return true after resetting a stale conversation to the vault. Callers
 *  must not consume/send the current composer text in that branch. */
export async function preflightChatContext(): Promise<boolean> {
  const previousCwd = await api.chatPreflight(chatCwd() ?? undefined);
  if (!previousCwd) return false;
  recoverChatContext(previousCwd);
  return true;
}

export interface ChatSendProps { workspace?: boolean }

export interface ChatSendDeps {
  /** The composer's textarea (assigned during render: a getter). */
  textarea: () => ComposerField | undefined;
  /** The transcript follows a send to its end (chatScrollController.ts). */
  followSend: () => void;
  /** Run a local slash command; false when it isn't one (slashCommands.ts). */
  handleLocalCommand: (prompt: string) => boolean;
  /** The composer's suggested next message (ChatBody). */
  suggesting: () => boolean;
  suggestion: () => ShipAction | null;
  placeSuggestion: (text: string) => void;
  /** The tag of a request Hollow drafted into the composer, when `prompt`
   *  is that draft unchanged (`cannedSend`); the composer's own send only. */
  cannedFor?: (prompt: string) => CannedRequest | undefined;
}

export function createChatSend(props: ChatSendProps, deps: ChatSendDeps) {
  const { textarea, followSend, handleLocalCommand, suggesting, suggestion, placeSuggestion, cannedFor } = deps;
  // Your work's Ask anything (2026-09-30) starts a quick chat and leaves its
  // first message here: send it through this composer's own send path once
  // the new conversation is on screen and idle.
  createEffect(() => {
    const pending = pendingChatSend();
    if (!pending || !props.workspace || chatCwd() !== pending.cwd || chatBusy() || chatSessionLoading()) return;
    if (pending.sessionKey !== undefined && chatAgentSessionKey() !== pending.sessionKey) return;
    setPendingChatSend(null);
    const done = (ok: boolean) => pending.done?.(ok);
    if (pending.preset) { queueMicrotask(() => void send(undefined, pending.text, pending.keepFocus, pending.canned).then(done, () => done(false))); return; }
    setChatInput(pending.text);
    queueMicrotask(() => void send(undefined, undefined, pending.keepFocus).then(done, () => done(false)));
  });

  /** `preset` sends those words as if typed (a ship button), leaving the
   *  composer's draft and attachments where they are. `keepFocus` leaves
   *  focus where it is once queued (a section's Make a canvas view). */
  /** Resolves true once a turn was queued (question cards, 2026-10-14: an
   *  ask-canvas's Send says "Sent." only then). `canned` tags a preset
   *  Hollow wrote (overnight 9 A1). */
  async function send(e?: Event, preset?: string, keepFocus = false, canned?: CannedRequest, box = true): Promise<boolean> {
    e?.preventDefault();
    // A send ends dictation into this box at once (P1-55): no late word lands
    // in the emptied box, and the microphone goes off.
    const field = textarea();
    if (!preset && field) dictationSent(field);
    // The send button on an empty composer sends the suggested next step, or
    // after a reply that read outside content, places it for review (D2).
    if (preset == null && suggesting()) {
      if (suggestion()!.review) { placeSuggestion(suggestion()!.prompt); return false; }
      preset = suggestion()!.prompt;
    }
    const prompt = preset ?? chatInput().trim();
    if (!preset && prompt === '/stop' && chatBusy()) { setChatInput(''); stop(); return false; }
    const atts = preset ? [] : attachments().filter(attachmentSendable);
    // Local commands first (P1-30): while a turn runs one never steers into
    // the agent. One that changes the next turn waits, its text kept.
    const local = !preset && chatBusy() && attachments().length === 0 ? busyLocalCommand(prompt) : null;
    if (local === 'refuse') { pushChatTurn({ role: 'system', text: busyCommandRefusal(prompt), tools: [] }); return false; }
    if (local === 'run') { handleLocalCommand(prompt); setChatInput(''); queueMicrotask(() => textarea()?.focus()); return false; }
    // A send of this chat still in its preflight holds the box's words: a
    // second Enter now would send them twice (ENG-73).
    if (!preset && preparingSendIn(chatAgentSessionKey())) return false;
    // Mid-turn steering: a send while this conversation's turn runs is
    // Steer now, Queue for after or Stop and send (`busyChoice`).
    if (!preset && chatBusy() && !chatSessionLoading() && (prompt || atts.length) && !attachments().some(a => a.saving || a.error)) {
      return sendWhileBusy(busyChoice());
    }
    if ((!prompt && atts.length === 0) || chatBusy() || chatSessionLoading() || (!preset && attachments().some(a => a.saving || a.error))) return false;

    if (prompt) {
      pushChatHistory(prompt);
      setChatHistoryCursor(null);
    }

    // Slash commands only apply to a bare text prompt (no attachments).
    if (!preset && atts.length === 0 && handleLocalCommand(prompt)) {
      setChatInput('');
      queueMicrotask(() => textarea()?.focus());
      return false;
    }

    // From here until `chat_send` answers, a Stop is this send's own (C10):
    // it is marked on `prepared`, and nothing is queued once it is.
    const prepared = beginPreparingSend(newClientTurnId(), chatAgentSessionKey());
    // Words never taken from the box stay there; a preset's go in beside
    // any draft (an ask-canvas's answers stay in their canvas).
    const stoppedHere = () => {
      if (preset != null && box) setChatInput(restoredDraft(preset, chatInput()));
      return stoppedBeforeSend(prepared);
    };
    // A fast first send waits for discovery before deciding availability.
    // Through every await until the turn is queued, the conversation stays
    // on screen (`holdConversationForSend`): left meanwhile, these words
    // would go to the conversation opened next (run 15 P3 second-pass review).
    if (agentDescriptors().length === 0) {
      setChatBusy(true);
      const release = holdConversationForSend();
      try { await refreshAgentSettings(true); } catch (err) { endPreparingSend(prepared); throw err; } finally { setChatBusy(false); release(); }
      if (prepared.stopped) return stoppedHere();
    }
    if (agentSettingsError() || !agentsAvailable().includes(chatAgent())) {
      endPreparingSend(prepared);
      unavailableAgentNotice(agentSettingsError() || `${AGENT_LABELS[chatAgent()]} is unavailable. Check Agent updates, then try again.`);
      return false;
    }

    // Verify the persisted cwd before consuming the composer or building
    // page context. A deleted worktree resets the conversation but
    // leaves the message in place so the user can make it self-contained and
    // explicitly send it again.
    setChatBusy(true);
    const release = holdConversationForSend();
    try {
      if (await preflightChatContext()) {
        endPreparingSend(prepared);
        setChatBusy(false);
        queueMicrotask(() => textarea()?.focus());
        return false;
      }
    } catch (err) {
      endPreparingSend(prepared);
      // An unsafe working folder (Paperclip PR 19) keeps the conversation,
      // its folder and the composer text; never the vanished-folder reset.
      const unsafe = unsafeCwdText(err);
      const msg = err instanceof Error ? err.message : String(err);
      pushChatTurn({ role: 'system', text: unsafe ? unsafeCwdNotice(unsafe) : `(error: could not verify the chat working folder: ${msg})`, tools: [] });
      setChatBusy(false);
      if (unsafe) queueMicrotask(() => textarea()?.focus());
      return false;
    } finally { release(); }
    // Stopped while it got ready: nothing is queued, and the words and
    // attachments were never taken from the box.
    if (prepared.stopped) return stoppedHere();

    // Preserve anything typed while the cheap preflight was in flight.
    if (!preset) {
      if (chatInput().trim() === prompt) setChatInput('');
      setAttachments(current => current.filter(a => !atts.some(sent => sent.id === a.id)));
    }
    // UI shows the user's own text + image thumbnails; the agent receives
    // the request with this send's own attachments: the <page-context> block
    // and any attached image paths (it Reads them). The conversation bundle
    // and the operator profile are the backend's (context_compiler.rs, agent
    // effectiveness F4): it adds both whenever the turn starts a fresh
    // native session, for this composer and for sessions agents start.
    const payload = composePayload(prompt, atts);
    // A drafted request sent as drafted keeps Hollow's tag; a preset that
    // isn't Hollow's (a ship button, the suggested next step) has none.
    const tag = preset != null ? canned : atts.length ? undefined : cannedFor?.(prompt);
    return dispatchTurn({ display: prompt, payload, atts, keepFocus, canned: tag, box }, prepared);
  }

  /** A send Stop reached before anything was queued (C10): the chat says so
   *  and is idle again. Always false (nothing was sent). */
  function stoppedBeforeSend(prepared: PreparingSend): false {
    endPreparingSend(prepared);
    pushChatTurn({ role: 'system', text: STOPPED_BEFORE_SENT, tools: [] });
    setChatBusy(false);
    queueMicrotask(() => textarea()?.focus());
    return false;
  }

  /** What the agent receives for a composer message: the page context
   *  (outside a workspace), any page the person shared from Annotate (one
   *  untrusted `<shared-page>` block each, repo ideas 2026-10-27 #3),
   *  Hollow's intent line, the words, any images and any other files
   *  (C5: one `<attached-files>` line each, its name and saved path). */
  function composePayload(prompt: string, atts: ChatAttachment[]): string {
    const ctx = !props.workspace && chatContextEnabled() ? pageContext() : null;
    const ctxPart = ctx
      ? `<page-context>\n${ctx.body}\n\n(This context is auto-injected from the user's current Hollow tab. Use it only if relevant to their message; do not echo it back.)\n</page-context>\n\n`
      : '';
    const pagePart = atts.flatMap(a => (a.page ? [`${sharedPageBlock(a.page)}\n\n`] : [])).join('');
    const intentPart = `<hollow-chat-intent>\nInfer the appropriate approach from the user's current request: answer, act, review, or automate as needed. Adapt as the conversation changes. Previous UI mode selections are historical and do not constrain this request. Respect configured permissions and confirmation requirements.\n</hollow-chat-intent>\n\n`;
    const images = atts.filter(a => !a.page && !a.file);
    const imgPart = images.length
      ? `\n\n<attached-images count="${images.length}">\nThe user attached ${images.length} image${images.length > 1 ? 's' : ''} to this message. Open ${images.length > 1 ? 'them' : 'it'} using your available image-viewing tool:\n${images.map((a, i) => `${i + 1}. ${a.path}`).join('\n')}\n</attached-images>`
      : '';
    const files = attachedFilesBlock(atts.filter(a => a.file));
    return `${ctxPart}${pagePart}${intentPart}${prompt}${imgPart}${files ? `\n\n${files}` : ''}`;
  }

  // ── Mid-turn steering (2026-10-21, chatSteer.ts) ──
  // While a turn runs the composer stays open. Enter does the default
  // (Steer now where this agent takes messages mid-turn, else Queue for
  // after); the Send options menu picks another for this one send.
  ensureChatSteerSupport();
  // The running turn's agent (repo ideas 10-27 #8: a continued turn may run
  // on another), not only the one picked for the next send.
  const steerAgent = () => steeringAgent(chatAgent());
  const steerNative = () => steersNatively(steerAgent());
  const hasImages = () => attachments().some(a => !a.page && !a.file);
  const hasFiles = () => attachments().some(a => !!a.file);
  const hasPages = () => attachments().some(a => !!a.page);
  // What rules Steer now out (P1-30): an image, a draft past the backend's
  // limit, a turn still queued, or support not read yet. Enter queues then,
  // and the hint says why.
  // Measured only while a turn runs, once per change.
  const steerBlock = createMemo(() => ({ tooLong: chatBusy() && steerTooLong(chatInput()), queued: isVisiblyQueued(chatQueuedTurn()) || isPreparingTurn(chatQueuedTurn()), unknown: !chatTurnSteerSupport() && !chatSteerSupport() }));
  // A failed support read is asked again when a turn starts (review), so
  // "checking" never sticks.
  createEffect(() => { if (chatBusy() && !chatSteerSupport()) ensureChatSteerSupport(); });
  const steerWhy = createMemo(() => steerBlockedWhy(AGENT_LABELS[steerAgent() as keyof typeof AGENT_LABELS] ?? 'The agent', steerNative(), { ...steerBlock(), images: hasImages(), files: hasFiles(), pages: hasPages() }));
  const defaultChoice = createMemo<BusyChoice>(() => defaultBusyChoice(steerNative(), hasImages(), { ...steerBlock(), files: hasFiles(), pages: hasPages() }));
  const [pickedChoice, setPickedChoice] = createSignal<BusyChoice | null>(null);
  const busyChoice = () => {
    const picked = pickedChoice();
    // A picked Steer now that something rules out falls back to the default.
    return picked && !(picked === 'steer' && steerWhy()) ? picked : defaultChoice();
  };
  // A turn that ends drops the one-send pick.
  createEffect(() => { if (!chatBusy()) setPickedChoice(null); });

  /** Send the composer's message with `choice` while a turn runs. Every
   *  setting is read now, before any await: if the person opens another
   *  conversation meanwhile, the reply and any fallback still belong to
   *  this one (review). */
  async function sendWhileBusy(choice: BusyChoice): Promise<boolean> {
    const prompt = chatInput().trim();
    const atts = attachments().filter(attachmentSendable);
    const ctx = busyContext();
    if (!ctx.key || (!prompt && atts.length === 0)) return false;
    setPickedChoice(null);
    if (prompt) { pushChatHistory(prompt); setChatHistoryCursor(null); }
    setChatInput('');
    setAttachments(current => current.filter(a => !atts.some(sent => sent.id === a.id)));
    if (choice === 'steer' && prompt && atts.length === 0) {
      try {
        const event = await api.chatSteer(ctx.key, prompt, newSteerId());
        // On screen still: draw it now (the bus event would too, later).
        if (onScreen(ctx)) { applyOwnSteer(event.steer); followSend(); queueMicrotask(() => textarea()?.focus()); }
        return true;
      } catch (err) {
        // In words, never the code (P1-30): a refusal Hollow made comes back
        // to the box; a turn not started, finishing or unable to take it
        // sends the words as the next message, and the chat says which.
        const refusal = steerRefusalNotice(steerRefusal(err), AGENT_LABELS[ctx.agent]);
        if (!refusal.fallsBack) {
          giveBack(ctx, prompt, [], refusal.text);
          return false;
        }
        if (onScreen(ctx)) pushChatTurn({ role: 'system', text: refusal.text, tools: [] });
        return holdOrSend(ctx, prompt, [], 'queue');
      }
    }
    return holdOrSend(ctx, prompt, atts, choice === 'stop' ? 'stop' : 'queue');
  }

  /** A steer the agent didn't take, sent again as the next message (P1-30):
   *  held for after a turn that still runs, else an ordinary send. The box
   *  keeps whatever it holds. */
  async function sendAsNext(text: string): Promise<boolean> {
    if (!chatBusy()) return send(undefined, text);
    const ctx = busyContext();
    return ctx.key ? holdOrSend(ctx, text, [], 'queue') : false;
  }

  /** The conversation a busy send belongs to, read once. */
  function busyContext() {
    return {
      key: chatAgentSessionKey(), sessionId: chatSessionId(), cwd: chatCwd(), model: chatModel(), mode: chatPermissionMode(),
      agent: chatAgent(), ultracode: chatUltracode(), canvases: chatCanvases(), continuedFrom: chatContinuedFrom(), effort: chatEffort(),
    };
  }
  type BusyContext = ReturnType<typeof busyContext>;
  const onScreen = (ctx: BusyContext) => !!ctx.key && chatAgentSessionKey() === ctx.key;

  /** Words (and images) that weren't sent go back in the box, beside
   *  anything typed since; in another conversation, a footer note says so. */
  function giveBack(ctx: BusyContext, prompt: string, atts: ChatAttachment[], why: string) {
    returnToConversation(ctx.key, prompt, atts, `${why.trim().replace(/[.\s]+$/, '')}. It's back in the box.`);
  }

  /** Queue for after / Stop and send: held by the backend while the turn
   *  runs (a chip under the reply), or sent now when it already ended. A
   *  held send's outcome that beat this reply (`takeEarlyHeld`), and a sent
   *  turn's events that did (`adoptBusySend`), are reconciled here. */
  async function holdOrSend(ctx: BusyContext, prompt: string, atts: ChatAttachment[], after: 'queue' | 'stop', answers?: (held: string) => void): Promise<boolean> {
    // An ask-canvas's answers (`answers`, P1-33) never come back to the box:
    // the canvas keeps them and offers Send now; the chat says why.
    const back = (what: string, reason: string) => {
      if (!answers) giveBack(ctx, prompt, atts, `${what} wasn't sent: ${reason}`);
      else if (onScreen(ctx)) pushChatTurn({ role: 'system', text: `Your answers weren't sent: ${reason.trim().replace(/[.\s]+$/, '')}. They're still in the canvas.`, tools: [] });
    };
    const clientTurnId = newClientTurnId();
    // Registered first: its turn's events may beat this reply.
    registerBusySend(clientTurnId, ctx.key, prompt, atts);
    try {
      const accepted = await api.chatSend(composePayload(prompt, atts), ctx.sessionId, ctx.cwd ?? undefined, ctx.model, ctx.mode, ctx.agent, ctx.key, clientTurnId, ctx.ultracode, ctx.canvases, ctx.continuedFrom, cannedFor?.(prompt) ?? null, after, ctx.effort);
      if (accepted.forwardedFrom != null) {
        // This conversation continued elsewhere: the message went there.
        forgetBusySend(clientTurnId);
        const early = accepted.heldId ? takeEarlyHeld(accepted.heldId) : null;
        if (early?.type === 'heldFailed') { back('Your waiting message', early.error); return false; }
        if (accepted.heldId && !early) addHeldChip(accepted.sessionKey, { id: accepted.heldId, text: prompt, after: 'queue', attachments: atts });
        followForward(accepted);
        return true;
      }
      const early = accepted.heldId ? takeEarlyHeld(accepted.heldId) : null;
      // A held send's own turn is reconciled through its chip, not here.
      if (accepted.heldId) forgetBusySend(clientTurnId);
      if (accepted.heldId && !early) {
        // Its words already came back (a reply later than the failure's
        // grace): not sent, and an ask-canvas's answers stay in their canvas.
        if (!addHeldChip(accepted.sessionKey, { id: accepted.heldId, text: prompt, after, attachments: atts })) return false;
        answers?.(accepted.heldId);
      } else if (early?.type === 'heldFailed') {
        back('Your waiting message', early.error);
        return false;
      } else if (onScreen(ctx)) {
        // Sent now (the turn had ended), or held and already gone.
        const queued = early?.type === 'heldSent' ? { ...accepted, queueId: early.queueId } : accepted;
        adoptBusySend(clientTurnId, prompt, atts, queued);
      }
      if (onScreen(ctx)) { followSend(); queueMicrotask(() => textarea()?.focus()); }
      return true;
    } catch (err) {
      forgetBusySend(clientTurnId);
      back('Your message', errMsg(err));
      return false;
    }
  }

  /** An ask-canvas's answers (P1-33): sent now, or while this conversation's
   *  turn runs held for after it (Queue for after, never Steer), with
   *  `held` told the held send's id. `locked` is read again just before the
   *  send goes (an answer that landed meanwhile wins). They never go to the
   *  message box: true once sent or held, else false. */
  async function sendAnswers(text: string, locked: () => boolean, held: (id: string) => void): Promise<boolean> {
    if (locked()) return false;
    if (!chatBusy()) return send(undefined, text, false, undefined, false);
    const ctx = busyContext();
    return ctx.key && !chatSessionLoading() ? holdOrSend(ctx, text, [], 'queue', held) : false;
  }

  /** Send the composer's message with `choice` now (a Send options item). */
  function sendWith(choice: BusyChoice) {
    setPickedChoice(choice);
    return send();
  }

  /** Send one turn through the queued send path (Paperclip PR 7b), after the
   *  caller set `chatBusy` and ran the working-folder preflight. `display` is
   *  the user bubble, `payload` what the agent receives. An answer turn
   *  (Paperclip PR 13) carries its card: if nothing could be queued, the card
   *  comes back, or its answers go to the composer, never lost. Shared with
   *  later callers that send a raw payload (PR 14's Continue). `keepFocus`
   *  leaves focus where it is once queued, instead of in the composer. */
  async function dispatchTurn(turn: {
    display: string;
    payload: string;
    atts: ChatAttachment[];
    interactionId?: string;
    keepFocus?: boolean;
    canned?: CannedRequest;
    /** False for an ask-canvas's answers: a failure never puts them in the box. */
    box?: boolean;
  }, prepared: PreparingSend): Promise<boolean> {
    const { display, payload, atts } = turn;
    // The send only ENQUEUES (Paperclip PR 7b). The nonce ties this bubble to
    // the turn's events, and the queued-turn record lets Edit, Discard, or an
    // end before start put the text and images back. It is the one the
    // caller registered as getting ready: a Stop from here until `chat_send`
    // answers is this send's (C10).
    const { clientTurnId } = prepared;
    prepared.queued = true;
    pushChatTurn({ role: 'user', text: display, tools: [], images: attachmentImages(atts), ...turnFilesField(atts), sharedPage: atts.some(a => !!a.page) || undefined, clientTurnId, at: new Date().toISOString() });
    beginQueuedChatTurn({
      clientTurnId, text: display, attachments: atts, requestedCwd: chatCwd(), sessionKey: chatAgentSessionKey(),
      interactionId: turn.interactionId ?? null,
    });
    followSend();
    if (!turn.keepFocus) queueMicrotask(() => textarea()?.focus());
    const requestedCwd = chatCwd();
    // The conversation this send belongs to (C8a): once queued-but-unanswered
    // it may be left for another (it goes to the background), and the reply
    // or error is still this one's. A fresh chat has no key and can't be left.
    const key = chatAgentSessionKey();
    const away = () => !!key && chatAgentSessionKey() !== key && chatQueuedTurn()?.clientTurnId !== clientTurnId;
    try {
      let accepted: SendAccepted | null;
      try {
        accepted = await api.chatSend(payload, chatSessionId(), chatCwd() ?? undefined, chatModel(), chatPermissionMode(), chatAgent(), chatAgentSessionKey(), clientTurnId, chatUltracode(), chatCanvases(), chatContinuedFrom(), turn.canned ?? null, null, chatEffort());
      } finally { endPreparingSend(prepared); }
      if (prepared.stopped && accepted) {
        // Stopped while `chat_send` was in flight (C10): its accepted row is
        // withdrawn now. Still queued on screen, it ends as Stop on a queued
        // turn does; forwarded or left, it ends like a send that failed.
        const here = accepted.forwardedFrom == null && !away();
        if (here) acceptQueuedChatTurn(clientTurnId, accepted);
        const outcome = await withdrawStoppedSend(clientTurnId, accepted);
        if (outcome === 'withdrawn') {
          if (here) return false;
          throw new Error(STOPPED_REASON);
        }
        // Forwarded and past stopping (Codex review): no chip, no follow, as
        // if sent normally; the chat says what became of it.
        if (accepted.forwardedFrom != null && outcome !== 'failed') return stoppedTooLate(away() ? key : null, clientTurnId, outcome === 'stopped');
        // Started here (its row stopped it), or a held failure parked for
        // the forwarded path: an ordinary reply below.
      }
      if (away()) return sentAway(key!, clientTurnId, accepted, turn);
      if (accepted?.forwardedFrom != null) {
        // One conversation (2026-11-07): this one continued elsewhere (or a
        // new Main agent thread joined the working one), and the message went
        // to the chain's current conversation. Its bubble moves there.
        batch(() => { abandonQueuedChatTurn(clientTurnId); removeChatUserTurn({ clientTurnId, queueId: null }); setChatBusy(false); });
        const early = accepted.heldId ? takeEarlyHeld(accepted.heldId) : null;
        if (early?.type === 'heldFailed') {
          const reason = early.error.trim().replace(/[.\s]+$/, '');
          // A card's answers reopen its card; a canvas's stay in the canvas
          // (box false). Neither goes into the box.
          if (turn.interactionId) {
            pushChatTurn({ role: 'system', text: `Your answers were not sent: ${reason}.`, tools: [] });
            await returnUnsentAnswers(turn.interactionId, display);
            return false;
          }
          if (turn.box === false) {
            pushChatTurn({ role: 'system', text: `Your answers weren't sent: ${reason}. They're still in the canvas.`, tools: [] });
            return false;
          }
          pushChatTurn({ role: 'system', text: `Your message wasn't sent: ${reason}.`, tools: [] });
          // Words and attachments both come back, beside anything typed
          // since (PR #1391 review).
          setChatInput(restoredDraft(display, chatInput()));
          setAttachments(current => [...atts, ...current.filter(item => !atts.some(sent => sent.id === item.id))]);
          return false;
        }
        if (accepted.heldId && !early) addHeldChip(accepted.sessionKey, { id: accepted.heldId, text: display, after: 'queue', attachments: atts });
        followForward(accepted);
        return true;
      }
      // runStarted may already have arrived; the store then shows no badge.
      if (accepted) acceptQueuedChatTurn(clientTurnId, accepted);
      return true;
    } catch (err) {
      if (away()) { failedAway(key!, clientTurnId, err, turn); return false; }
      abandonQueuedChatTurn(clientTurnId);
      const msg = err instanceof Error ? err.message : String(err);
      if (turn.interactionId) {
        // Nothing was queued for an answer turn. Its bubble goes; the card
        // is reopened (or the answers land in the composer).
        removeChatUserTurn({ clientTurnId, queueId: null });
        if (msg.startsWith(CHAT_CONTEXT_RECOVERED_PREFIX)) recoverChatContext(requestedCwd ?? 'the selected folder');
        pushChatTurn({ role: 'system', text: `Your answers were not sent: ${msg.trim().replace(/[.\s]+$/, '')}.`, tools: [] });
        setChatBusy(false);
        await returnUnsentAnswers(turn.interactionId, display);
        return false;
      }
      if (msg.startsWith(CHAT_CONTEXT_RECOVERED_PREFIX)) {
        // The cwd vanished between preflight and enqueue; nothing was queued.
        // Retract the unsent row, restore the composer, and require another
        // explicit send. Attachment files may have vanished with the
        // worktree, so mark them failed instead of handing out stale paths.
        removeChatUserTurn({ clientTurnId, queueId: null });
        recoverChatContext(requestedCwd ?? 'the selected folder');
        // Beside anything typed since, never instead of it (PR #1391 review).
        if (turn.box !== false) setChatInput(restoredDraft(display, chatInput()));
        setAttachments(current => [
          // A shared page holds its text, not a file: it stays as it was.
          ...atts.map(a => (a.page ? a : { ...a, path: undefined, saving: false, error: true })),
          ...current.filter(item => !atts.some(sent => sent.id === item.id)),
        ]);
        setChatBusy(false);
        queueMicrotask(() => textarea()?.focus());
        return false;
      }
      // Nothing was queued (a turn already waiting, a queue this window does
      // not own, a lease only a restart clears, an unsafe working folder):
      // the message was never sent, so it goes back to the composer with the
      // reason. An unsafe folder (Paperclip PR 19) keeps the conversation and
      // its folder; the notice says how to pick another.
      removeChatUserTurn({ clientTurnId, queueId: null });
      // An ask-canvas's answers (`box` false, P1-33) stay in their canvas.
      // Beside anything typed since (a blank line between), never dropped:
      // the chat may have been left and reopened meanwhile (PR #1391 review).
      const restored = turn.box !== false;
      if (restored) setChatInput(restoredDraft(display, chatInput()));
      setAttachments(current => [...atts.filter(a => !current.some(c => c.id === a.id)), ...current]);
      const unsafe = unsafeCwdText(err);
      const notice = turn.box === false ? `Your answers weren't sent: ${msg.trim().replace(/[.\s]+$/, '')}. They're still in the canvas.` : queuedFailureNotice(msg, restored);
      pushChatTurn({ role: 'system', text: unsafe ? unsafeCwdNotice(unsafe) : notice, tools: [] });
      setChatBusy(false);
    }
      return false;
  }

  type DispatchedTurn = Parameters<typeof dispatchTurn>[0];

  /** A forwarded send Stop reached too late (C10, Codex review): it started
   *  in the chain's current conversation and its row's cancel ended it
   *  (`stopped`, false), or it had already gone and nothing was stopped
   *  (true: it was sent). Its bubble leaves the conversation it was sent
   *  from (`awayKey` once left), which says so; never a chip or a follow. */
  function stoppedTooLate(awayKey: string | null, clientTurnId: string, stopped: boolean): boolean {
    const notice = stopped ? STOPPED_AFTER_START : ALREADY_SENT;
    if (awayKey) endBackgroundSend(awayKey, clientTurnId, notice);
    else {
      batch(() => { abandonQueuedChatTurn(clientTurnId); removeChatUserTurn({ clientTurnId, queueId: null }); setChatBusy(false); });
      pushChatTurn({ role: 'system', text: notice, tools: [] });
    }
    return !stopped;
  }

  /** `chat_send` answered after the person left the conversation `key` it
   *  was sent from (C8a): the reply goes to that conversation's background
   *  entry, and the conversation on screen (its box, transcript and busy
   *  state) is left alone. */
  function sentAway(key: string, clientTurnId: string, accepted: SendAccepted | null, turn: DispatchedTurn): boolean {
    if (accepted?.forwardedFrom == null) {
      if (accepted) acceptBackgroundSend(key, clientTurnId, accepted);
      return true;
    }
    // It went to the chain's current conversation: nothing waits here.
    const early = accepted.heldId ? takeEarlyHeld(accepted.heldId) : null;
    if (early?.type === 'heldFailed') {
      const reason = early.error.trim().replace(/[.\s]+$/, '');
      endBackgroundSend(key, clientTurnId, `Your message wasn't sent: ${reason}.`);
      returnToConversation(key, turn.box === false ? '' : turn.display, turn.box === false ? [] : turn.atts, turn.box === false ? `Your answers weren't sent: ${reason}. They're still in the canvas.` : `Your message wasn't sent: ${reason}.`);
      return false;
    }
    endBackgroundSend(key, clientTurnId, null);
    if (accepted.heldId && !early) addHeldChip(accepted.sessionKey, { id: accepted.heldId, text: turn.display, after: 'queue', attachments: turn.atts });
    const title = accepted.forwardedTitle?.trim();
    flashFooter(`A message from another chat went to ${title ? `“${title}”` : 'the current chat'}, where that conversation continues.`, 5000);
    return true;
  }

  /** `chat_send` failed after the person left the conversation `key` (C8a):
   *  that conversation stops working and gets the words back (stashed for
   *  its box, a footer note now); the one on screen is untouched. */
  function failedAway(key: string, clientTurnId: string, err: unknown, turn: DispatchedTurn) {
    const msg = err instanceof Error ? err.message : String(err);
    const reason = msg.trim().replace(/[.\s]+$/, '');
    if (turn.interactionId) {
      const notice = `Your answers were not sent: ${reason}.`;
      endBackgroundSend(key, clientTurnId, notice);
      void returnUnsentAnswersTo(key, turn.interactionId, turn.display, notice);
      return;
    }
    if (turn.box === false) {
      const notice = `Your answers weren't sent: ${reason}. They're still in the canvas.`;
      endBackgroundSend(key, clientTurnId, notice);
      returnToConversation(key, '', [], notice);
      return;
    }
    // A vanished folder took the saved files with it: offered as failed, as
    // on screen; that chat's next send resets it (its preflight).
    const atts = msg.startsWith(CHAT_CONTEXT_RECOVERED_PREFIX)
      ? turn.atts.map(a => (a.page ? a : { ...a, path: undefined, saving: false, error: true }))
      : turn.atts;
    const unsafe = unsafeCwdText(err);
    endBackgroundSend(key, clientTurnId, unsafe ? unsafeCwdNotice(unsafe) : queuedFailureNotice(msg));
    returnToConversation(key, turn.display, atts, `Your message wasn't sent: ${reason}.`);
  }

  // ── Manual Continue (Paperclip PR 14b) ──
  // Offered under a stopped, interrupted or stalled last turn. One click sends
  // exactly CONTINUE_PROMPT through the normal queued send, on the same
  // conversation (its key and native session), with no context blocks and
  // without touching the draft. Never automatic, and tools are never replayed.
  const continueOffered = () => canContinueChat(chatTurns, {
    busy: chatBusy() || chatSessionLoading(),
    queued: !!chatQueuedTurn(),
    sessionId: chatSessionId(),
  });

  async function continueRun() {
    if (!continueOffered()) return;
    if (agentSettingsError() || !agentsAvailable().includes(chatAgent())) {
      unavailableAgentNotice(agentSettingsError() || `${AGENT_LABELS[chatAgent()]} is unavailable. Check Agent updates, then try again.`);
      return;
    }
    setChatBusy(true);
    // A Stop while it gets ready is its own (C10), as for `send`.
    const prepared = beginPreparingSend(newClientTurnId(), chatAgentSessionKey());
    // On screen until the turn is queued, like `send` (run 15 P3).
    const release = holdConversationForSend();
    try {
      // The working folder vanished and the conversation was reset: there
      // is no session left to continue. Nothing was sent.
      if (await preflightChatContext()) {
        endPreparingSend(prepared);
        setChatBusy(false);
        return;
      }
    } catch (err) {
      endPreparingSend(prepared);
      const unsafe = unsafeCwdText(err);
      pushChatTurn({ role: 'system', text: unsafe ? unsafeCwdNotice(unsafe) : `(error: could not verify the chat working folder: ${errMsg(err)})`, tools: [] });
      setChatBusy(false);
      return;
    } finally { release(); }
    if (prepared.stopped) { stoppedBeforeSend(prepared); return; }
    await dispatchTurn({ display: CONTINUE_PROMPT, payload: CONTINUE_PROMPT, atts: [] }, prepared);
  }

  // ── Question cards (Paperclip PR 13) ──
  const [submittingCard, setSubmittingCard] = createSignal<string | null>(null);
  const [cardErrors, setCardErrors] = createSignal<Record<string, string>>({});
  function setCardError(id: string, message: string | null) {
    setCardErrors(current => {
      const next = { ...current };
      if (message) next[id] = message; else delete next[id];
      return next;
    });
  }

  /** Submit a card: the backend resolves it (target unchanged, answer valid)
   *  and returns the formatted answers, which are sent verbatim as the next
   *  turn with the conversation's session, key, cwd and agent, and no context
   *  blocks: into the session that asked when the conversation still resumes
   *  it, else as an ordinary message naming each question in full
   *  (2026-11-04), so a filled-in card always goes. */
  async function submitInteraction(card: ChatInteraction, answer: InteractionAnswer) {
    if (chatBusy() || chatSessionLoading() || submittingCard()) return;
    setCardError(card.id, null);
    setSubmittingCard(card.id);
    setChatBusy(true);
    // A Stop while it gets ready is its own (C10), as for `send`.
    const prepared = beginPreparingSend(newClientTurnId(), chatAgentSessionKey());
    // On screen until the answer turn is queued, like `send` (run 15 P3).
    const release = holdConversationForSend();
    try {
      try {
        if (await preflightChatContext()) {
          // The working folder vanished and the conversation was reset:
          // nothing was consumed (the reset drops this chat's cards).
          setChatBusy(false);
          return;
        }
      } catch (err) {
        setCardError(card.id, `Could not verify the chat working folder: ${errMsg(err)}`);
        setChatBusy(false);
        return;
      }
      // Stopped before the answers were taken: the card stays open.
      if (prepared.stopped) { stoppedBeforeSend(prepared); return; }
      let prompt: string;
      try {
        prompt = (await api.resolveInteraction(card.id, answer, chatSessionId())).prompt;
      } catch (err) {
        const msg = errMsg(err);
        setCardError(card.id, msg.replace(/^INTERACTION_[A-Z_]+:\s*/, ''));
        if (msg.startsWith('INTERACTION_STALE') || msg.startsWith('INTERACTION_CLOSED')) void refreshChatInteractions();
        setChatBusy(false);
        return;
      }
      removeChatInteraction(card.id);
      if (prepared.stopped) {
        // Stopped while they were taken: they come back to their card.
        stoppedBeforeSend(prepared);
        await returnUnsentAnswers(card.id, prompt);
        return;
      }
      release();
      await dispatchTurn({ display: prompt, payload: prompt, atts: [], interactionId: card.id }, prepared);
    } finally {
      endPreparingSend(prepared);
      release();
      setSubmittingCard(null);
    }
  }

  async function dismissInteraction(card: ChatInteraction) {
    setCardError(card.id, null);
    try {
      await api.dismissInteraction(card.id);
      removeChatInteraction(card.id);
    } catch (err) {
      setCardError(card.id, errMsg(err).replace(/^INTERACTION_[A-Z_]+:\s*/, ''));
      void refreshChatInteractions();
    }
  }

  // A queued turn is taken out of the queue (its text comes back); a running
  // one is stopped. Keyed, so a stale click can never stop another session.
  const stop = stopChatRun;

  /** The badge of a user bubble that has not started: Queued with its reason
   *  for a turn that truly waits, or just Edit and Discard while it is
   *  Preparing (2026-11-16), or null. */
  const queuedBadge = (t: ChatTurn) => {
    const q = chatQueuedTurn();
    return (isVisiblyQueued(q) || isPreparingTurn(q)) && turnIsQueuedTurn(t, q) ? q : null;
  };

  // A single live status line while a turn runs: what the agent is doing
  // right now ("Reading foo.ts", "Running command", "Responding", "Thinking").
  const activity = createMemo<string | null>(() => {
    if (!chatBusy()) return null;
    // A permission hold reads Paused, as the dock says (2026-11-17).
    const q = chatQueuedTurn();
    if (isVisiblyQueued(q)) return q?.heldReason ? 'Paused' : 'Queued';
    // Admission, recall, retrieval and the agent's boot are working too.
    if (isPreparingTurn(chatQueuedTurn())) return 'Preparing';
    const last = chatTurns[chatTurns.length - 1];
    if (last && last.role === 'assistant') {
      const running = last.tools.find(t => t.status === 'running');
      if (running) return activeVerb(running);
      if (last.partial && last.text.trim()) return 'Responding';
    }
    // The backend's progress phase (Paperclip PR 15b) when no tool row says
    // more, e.g. after a reload or while a tool's row is already settled.
    return progressLabel(chatActivity()?.progress) ?? 'Thinking';
  });
  // "quiet Nm" beside the verb once the running agent has printed nothing
  // for two minutes (Paperclip PR 15b). A 15 s clock, only while busy.
  const [nowMs, setNowMs] = createSignal(Date.now());
  createEffect(() => {
    if (!chatBusy()) return;
    setNowMs(Date.now());
    const id = setInterval(() => setNowMs(Date.now()), 15_000);
    onCleanup(() => clearInterval(id));
  });
  // `nowMs` only re-runs the memo; the label reads the current time, so a
  // fresh activity tick is measured against now, not the last clock tick.
  const quiet = createMemo(() => {
    nowMs();
    return activity() ? quietLabel(chatActivity()?.lastEventAt, Date.now()) : null;
  });

  return {
    send,
    continueOffered, continueRun,
    submittingCard, cardErrors, submitInteraction, dismissInteraction,
    stop, queuedBadge,
    activity, quiet,
    busyChoice, defaultChoice, steerNative, steerWhy, sendWith, sendAsNext, sendAnswers,
  };
}

export type ChatSend = ReturnType<typeof createChatSend>;

/** What the chat says when Stop came before the send was queued (C10). */
const STOPPED_BEFORE_SENT = 'Stopped before it was sent.';
/** The reason a send Stop withdrew after `chat_send` gives where it ends like
 *  a failure (forwarded, or its chat left): "...was not sent: you stopped it." */
const STOPPED_REASON = 'you stopped it';
/** A forwarded send Stop ended after it started in the other chat. */
const STOPPED_AFTER_START = 'Stopped after it started in the chat it went to.';
/** A forwarded send Stop came too late for (or couldn't find). */
const ALREADY_SENT = 'Stop came too late. It had already been sent.';

function shortName(p: string): string {
  const a = p.split('/').filter(Boolean);
  return a[a.length - 1] || p;
}

/** Present-tense label for the live activity line from the running tool. */
function activeVerb(t: ChatToolUse): string {
  const i = t.input && typeof t.input === 'object' ? (t.input as Record<string, unknown>) : {};
  const fp = typeof i.file_path === 'string' ? shortName(i.file_path) : '';
  switch (t.name) {
    case 'Read': return fp ? `Reading ${fp}` : 'Reading';
    case 'Edit': case 'MultiEdit': case 'NotebookEdit': return fp ? `Editing ${fp}` : 'Editing';
    case 'Write': return fp ? `Writing ${fp}` : 'Writing';
    case 'Bash': return 'Running command';
    case 'Grep': case 'Glob': return 'Searching files';
    case 'WebFetch': case 'WebSearch': return 'Searching the web';
    case 'Task': return 'Running a subagent';
    case 'TodoWrite': return 'Updating the plan';
    default:
      if (t.name.includes('hollow_team__ask_antigravity')) return 'Delegating to Antigravity';
      if (t.name.includes('hollow_team__ask_codex')) return 'Delegating to Codex';
      return t.name.startsWith('mcp__') ? 'Calling a connector' : `Running ${t.name}`;
  }
}

/** A send the backend moved to the chain's current conversation: the lazy
 *  chain bar says so and opens it (one Main conversation, 2026-11-07). */
function followForward(accepted: SendAccepted) {
  void import('../ChatChainBar').then(m => m.followForward(accepted)).catch(() => { /* the next refresh shows it */ });
}
