// Ask (consolidation D, 2026-10-13): asking Hollow with an explicit scope
// (askScopes.ts). Your work's Ask field (StudioHome.tsx) runs it. The top
// bar's Ask ⌄ panel, which also ran it, was removed on 2026-10-21: its other
// places each have their own button (the workspace's message box, the orb,
// New → New quick chat). One Ask (2026-11-03): Home's field is the one Ask,
// and every question is a chat; the palette's sentence row, its Ask
// anything… action and `g v` all come here.
import { createEffect, createRoot, createSignal, on } from 'solid-js';
import { ASK_SCOPES, defaultAskScope, type AskScope } from '../askScopes';
import { canLeaveConversation, chatAgentSessionKey, chatBusy, chatCwd, flashFooter, pendingChatSend, setChatPrefillRequest, setPendingChatSend, requestChatFocus } from './runtime';
import { askProject, goToPage, quickAskDraft, sendQuickAsk, setQuickAskDraft, workspaces } from './workspaces';
import { mainAgentOpen, openMainAgent } from './mainAgent';

const LAST_KEY = 'hollow.ask.lastScope';
/** A saved choice: the two scopes One Ask removed map to the chat nearest
 *  them, Answer from my notes to a quick chat and Plan it in Hollow to the
 *  Main agent, so a person who used them still starts where they would. */
const SAVED: Readonly<Record<string, AskScope>> = { main: 'main', quick: 'quick', notes: 'quick', hollow: 'main' };
const stored = (): AskScope | null => { try { const v = localStorage.getItem(LAST_KEY); return v != null && Object.hasOwn(SAVED, v) ? SAVED[v] : null; } catch { return null; } };
const [lastScope, setLastScope] = createSignal<AskScope | null>(stored());
export const lastAskScope = lastScope;

// The removed panel kept an unsent draft here: its text shows in Your work's
// Ask field instead (2026-10-21). The saved copy stays until the field moves
// on (sent, cleared or edited), so a reload before then doesn't lose it.
const OLD_DRAFT_KEY = 'hollow.ask.draft';
const dropOldDraft = () => { try { localStorage.removeItem(OLD_DRAFT_KEY); } catch { /* gone with the session */ } };
try {
  const old = localStorage.getItem(OLD_DRAFT_KEY);
  const text = old == null ? null : (JSON.parse(old) as { text?: unknown } | null)?.text;
  if (typeof text === 'string' && text.trim()) {
    setQuickAskDraft(text);
    createRoot(dispose => createEffect(on(quickAskDraft, v => { if (v !== text) { dropOldDraft(); dispose(); } }, { defer: true })));
  } else if (old != null) dropOldDraft();
} catch { dropOldDraft(); }

/** Home's Ask anything scope: its text is `quickAskDraft` (store/workspaces),
 *  and its chosen place survives leaving Home the same way. */
export const [homeAskScope, setHomeAskScope] = createSignal<AskScope | null>(null);
/** Where Home's Ask sends now: the place chosen there, else the last one. */
export const currentAskScope = (): AskScope => homeAskScope() ?? defaultAskScope(lastScope());
/** The scope a fresh Ask starts on next time: every submit writes it, a quick
 *  chat included (P1-11). */
export function rememberAskScope(scope: AskScope) {
  setLastScope(scope);
  try { localStorage.setItem(LAST_KEY, scope); } catch { /* session only */ }
}

/** Where `askHere` sends now, in the words Home's Ask shows: the scope's
 *  name, and for a quick chat the project its Add to project picked (the
 *  palette's Ask row names it, so it never sends into a project unseen). */
export function askPlaceLabel(): string {
  const where = currentAskScope();
  const label = ASK_SCOPES.find(s => s.id === where)?.label ?? where;
  const project = where === 'quick' ? askProject() : null;
  if (!project) return label;
  // New chat, or New chat in a project (Add to project).
  return `${label} in ${workspaces().find(w => w.id === project)?.title ?? 'an unavailable project'}`;
}

/** Set by `openHomeAsk`, taken by Home's Ask field once it is on screen. */
export const [homeAskFocusRequest, setHomeAskFocusRequest] = createSignal(false);
/** Your work with its Ask field focused (One Ask): `g v`, the palette's Ask
 *  anything… and the agent's `navigate` to `ask-vault` (the ids the removed
 *  Ask your vault had, kept so a custom key still works). `text` (what was
 *  typed in the palette) goes into the field only while it is empty, so an
 *  unsent draft stays. Both happen only once the navigation is accepted: a
 *  refused one (an unsaved edit kept) leaves nothing pending. */
export function openHomeAsk(text = '') {
  goToPage('today', { onNavigate: () => {
    if (text.trim() && !quickAskDraft().trim()) setQuickAskDraft(text.trim());
    setHomeAskFocusRequest(true);
  } });
}

/** The person's words, sent in the Main agent conversation now on screen
 *  once it is ready (run 15 P3): through the composer's own send, as typed
 *  words (`preset`, so the page's own draft stays), and bound to that
 *  conversation's key, so it never lands in another Main agent chat. Leaving
 *  that conversation before it goes withdraws it. Resolves true once the
 *  composer's send queued a turn, false when withdrawn, replaced or refused
 *  (an unavailable agent, a vanished folder): Home's Ask keeps the words. */
function sendOnMainPage(text: string, cwd: string): Promise<boolean> {
  return new Promise(resolve => {
    const req = { text, cwd, preset: true, sessionKey: chatAgentSessionKey(), done: resolve };
    if (chatBusy()) flashFooter('Sends when the COO finishes its current turn.', 4000);
    createRoot(dispose => {
      createEffect(on([pendingChatSend, mainAgentOpen, chatAgentSessionKey], ([pending, open, key]) => {
        // Taken by the composer (it answers through `done`), or replaced.
        if (pending !== req) { dispose(); if (pending) resolve(false); return; }
        if (!open || key !== req.sessionKey) { setPendingChatSend(null); dispose(); resolve(false); }
      }, { defer: true }));
      setPendingChatSend(req);
    });
  });
}

/** Send `text` to the Main agent on its page (run 15 P3 removed the drawer
 *  beside the page): from another page it is sent there (D2: the person
 *  typed it and pressed Enter; `sendOnMainPage`); with `place` (⌥-Enter), or
 *  asked from the page itself, it only fills the box. A quick chat is
 *  `askHere`'s `sendQuickAsk`. */
export async function runAsk(scope: Exclude<AskScope, 'quick'>, text: string, opts: { place?: boolean } = {}): Promise<boolean> {
  const message = text.trim();
  rememberAskScope(scope);
  const here = mainAgentOpen();
  if (!(await openMainAgent())) return false;
  const cwd = chatCwd();
  if (message && !opts.place && !here && cwd) return sendOnMainPage(message, cwd);
  if (message) setChatPrefillRequest(message);
  requestChatFocus();
  return true;
}

/** Ask `text` where Home's Ask is set to send (`currentAskScope`): Home's own
 *  submit and the palette's Ask row both come here, so the row never asks
 *  somewhere the field wouldn't. A quick chat (in the field's project, when
 *  one is picked) is `sendQuickAsk`, which clears the field only when it
 *  still holds these words. Resolves true once it went. */
export function askHere(text: string, opts: { place?: boolean } = {}): Promise<boolean> {
  // Home's button is greyed out meanwhile; the palette's row says why.
  if (!canLeaveConversation()) { flashFooter('Wait a moment: the current message is still being sent.', 3500); return Promise.resolve(false); }
  const where = currentAskScope();
  if (where === 'quick') { rememberAskScope(where); return sendQuickAsk(text); }
  return runAsk(where, text, opts);
}
