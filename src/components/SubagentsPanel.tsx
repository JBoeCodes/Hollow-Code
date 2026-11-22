import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { Icon } from '../icons';
import { AgentMark } from './AgentMark';
import { MarkdownView } from './MarkdownView';
import { openPreview, previewTarget } from '../store/preview';
import { flashFooter, openVaultHistory } from '../store/runtime';
import { showConfirm } from '../store/overlays';
import { enterSends } from '../store/prefs';
import { createSecondClock, createSubagentsRead } from '../store/subagents';
import { finishedText } from '../turnFooter';
import { SpeakerIcon } from './TurnFooter';
import { canVote, voteErrorText, voteSavedText } from '../replyActions';
import { ensureReplyVote, replyVotes, saveReplyVote } from '../store/replyFeedback';
import { readAloud, readAloudSupported, readingKey, stopReading } from '../store/readAloud';
import type { FeedbackVote } from '../types';
import { loadCatalog } from '../modelCatalog';
import { conversationPermission } from '../permModes';
import { newClientTurnId } from '../chatQueue';
import { errMsg } from '../utils';
import { agentGlyph, recentSteps, cfoHeaderLine, costWords, currentStep, groupWords, outcomeWords, finishedMark, workGroups, createIdentities, durationWords, findRow, olderLeft, headerMeta, latestCfo, replies, waitingWords, rowMeta, rowTitle, rowWhen, sentWords, stopSubagent, tickFrom, startedWords, type ModelNames } from '../subagentsModel';
import { NO_ROW_NOTICE } from '../runQueue';
import type { SubagentRow } from '../generated/ipc';
import type { AgentConversation } from '../types';
import '../styles/subagents.css';

const titleOf = (row: SubagentRow | null, fallback?: string | null) => row?.title?.trim() || fallback?.trim() || 'Untitled subagent';
const live = (row: SubagentRow | null) => row?.status === 'running' || row?.status === 'queued';
/** K3: a subagent Hollow started fresh for a small follow-up to a long one
 *  says so first on its row and header (`fresh_brief.rs` TRACE). */
const FRESH_TRACE = 'Fresh chat with a brief';
const fresh = (row: SubagentRow | null | undefined) => (row?.freshFrom ? [FRESH_TRACE] : []);
const freshTip = (row: SubagentRow) => (row.freshFrom ? `\n${FRESH_TRACE}, for a small follow-up to ${row.freshFrom}` : '');

/** A link in a subagent's transcript: a web page opens in the Preview pane;
 *  nothing navigates the window. */
function openLink(e: MouseEvent | KeyboardEvent) {
  e.preventDefault();
  e.stopPropagation();
  const href = (e.target as Element | null)?.closest('a')?.getAttribute('href') ?? '';
  if (/^https?:\/\//i.test(href)) openPreview(href);
}

/**
 * A chat's Subagents tab (COO rebuild L3, ChatGPT's subagents view with
 * Hollow's time and tokens): Active (running, then queued by place), Done
 * (newest first) and Earlier chats; a row opens that subagent's session in
 * the same tab, with a back arrow, its header, Stop while it works, its
 * transcript and a composer. The open subagent is the tab's target
 * (`open`), so Back and Forward in the pane walk it too. Reads while shown
 * (store/subagents.ts).
 */
export function SubagentsPanel(props: { parent: string }) {
  const read = createSubagentsRead(() => props.parent);
  const openKey = () => { const t = previewTarget(); return t?.kind === 'subagents' && t.parent === props.parent ? t.open ?? null : null; };
  const view = () => read.view();
  const groups = createMemo(() => workGroups([...(view()?.active ?? []), ...(view()?.done ?? [])]));
  const finishedGroups = createMemo(() => groups().filter(g => !g.active));
  // Stable within this chat; restarts when the panel switches chats.
  const numberFor = createIdentities();
  const identity = (key: string) => numberFor(props.parent, key);
  // Open teams, by root key: the <details> element is rebuilt when a team
  // moves between Active and Done, so the choice lives here, not in the DOM.
  const [openTeams, setOpenTeams] = createSignal<ReadonlySet<string>>(new Set<string>());
  createEffect(on(() => props.parent, () => setOpenTeams(new Set<string>()), { defer: true }));
  const anyLive = () => (view()?.running ?? 0) > 0;
  // Ticks every second while one runs; every read refreshes "16h ago" too.
  const tick = createSecondClock(anyLive);
  const now = () => { tick(); view(); return Date.now(); };
  // A cached read older than 30 s doesn't tick until a fresh one returns.
  const at = () => tickFrom(view()?.at ?? '', now());
  // Model names from each agent's own catalog, loaded once per agent.
  const [names, setNames] = createSignal<Record<string, ModelNames>>({});
  createEffect(() => {
    const v = view();
    for (const agent of new Set([...(v?.active ?? []), ...(v?.done ?? []), ...(v?.earlier ?? [])].map(r => r.agent))) {
      if (names()[agent]) continue;
      void loadCatalog(agent).then(c => setNames(n => ({ ...n, [agent]: { models: c.models, newestByFamily: c.newestByFamily } })));
    }
  });
  let list: HTMLDivElement | undefined;
  let listScroll = 0;
  let opened: string | null = null;
  const open = (row: SubagentRow) => {
    listScroll = list?.scrollTop ?? 0;
    opened = row.key;
    openPreview({ kind: 'subagents', parent: props.parent, open: row.key });
  };
  const back = () => openPreview({ kind: 'subagents', parent: props.parent });
  // Back to the list: its scroll place and the row that was opened.
  createEffect(on(openKey, (key, prev) => {
    if (key || !prev) return;
    requestAnimationFrame(() => {
      if (list) list.scrollTop = listScroll;
      // The pane's Back can land here from a handed-over conversation's key.
      const key = findRow(view(), opened ?? prev)?.key ?? opened ?? prev;
      const row = list?.querySelector<HTMLElement>(`[data-subagent="${CSS.escape(key)}"]`);
      for (let parent = row?.parentElement; parent && parent !== list; parent = parent.parentElement) {
        if (parent instanceof HTMLDetailsElement) parent.open = true;
      }
      row?.focus({ preventScroll: true });
    });
  }, { defer: true }));

  // Rows are keyed by conversation, so a poll (new objects every 3 s while
  // one runs) keeps each row's element, its hover and its focus. A row reads
  // shows public activity while live, then a compact recorded outcome.
  const Row = (p: { id: string }) => (
    <Show when={findRow(view(), p.id)}>{row => (
      <li>
        <button type="button" class="subagents-row" data-subagent={row().key} data-status={row().status} title={[rowTitle(row(), at(), now()) + freshTip(row()), startedWords(row()) ? `Started ${startedWords(row())}` : '', ...rowMeta(row(), at(), now(), names()[row().agent]).filter(m => m !== waitingWords(row())), costWords(row().costUsd), row().activity?.artifacts.length ? `Recorded files: ${row().activity?.artifacts.join(', ')}` : ''].filter(Boolean).join('\n')} onClick={() => open(row())}>
          <span class="subagents-mark" aria-hidden="true">{agentGlyph(row().key)}<span class="subagents-state-mark">{live(row()) ? identity(row().key) : finishedMark(row())}</span></span>
          <span class="subagents-row-title">{titleOf(row())}<Show when={startedWords(row())}>{words => <span class="subagents-row-by"> · {words()}</span>}</Show></span>
          <Show when={row().status === 'running'}><span class="sr-only">Running</span></Show>
          <span class="subagents-row-when">
            {live(row()) ? rowWhen(row(), at(), now()) : durationWords(row().workingMs)}
          </span>
          <Show when={!live(row())}><span class="subagents-row-state">{outcomeWords(row())}</span></Show>
          <Show when={live(row())}>
            <span class="subagents-current" title={currentStep(row())}>
              <span class="subagents-signal" aria-hidden="true"><i /><i /><i /></span>
              <Show when={currentStep(row())} keyed>{step => <span class="subagents-step-text">{step}</span>}</Show>
            </span>
            <Show when={row().status === 'running' && row().activity?.recent.length}>
              <span class="subagents-trail"><For each={recentSteps(row())}>{step => <span title={`${step.label} (${step.status})`}><span aria-hidden="true">{step.status === 'ok' ? '✓' : step.status === 'error' ? '!' : '·'}</span> {step.label}</span>}</For></span>
            </Show>
            <Show when={row().status === 'running' && row().activity?.progress}>{progress => <span class="subagents-plan"><progress max={progress().total} value={progress().completed} aria-label="Reported plan progress" /><span>{progress().completed} of {progress().total} reported steps</span></span>}</Show>
          </Show>
          <Show when={live(row()) && row().costUsd != null && Number.isFinite(row().costUsd)}><span class="subagents-cost">{costWords(row().costUsd)}</span></Show>
        </button>
      </li>
    )}</Show>
  );
  const keys = (rows: readonly SubagentRow[]) => rows.map(r => r.key);
  const Group = (p: { id: string }) => {
    const found = () => groups().find(g => g.id === p.id);
    return <Show when={found()}>{group => <Show when={group().kind !== 'single'} fallback={<ul class="subagents-rows"><For each={keys(group().rows)}>{id => <Row id={id} />}</For></ul>}>
      <details class="subagents-team" open={openTeams().has(group().teamKey)} onToggle={e => { const k = group().teamKey, was = openTeams().has(k); if (e.currentTarget.open === was) return; const n = new Set<string>(openTeams()); if (e.currentTarget.open) n.add(k); else n.delete(k); setOpenTeams(n); }}>
        <summary title={groupWords(group())}><span class="subagents-team-kind" aria-hidden="true" /><span class="sr-only">{group().kind === 'swarm' ? 'Swarm' : 'Team'}: </span><span class="subagents-team-title">{group().title}</span><span class="subagents-team-meta">{group().parts} parts<Show when={group().active}> · active</Show></span></summary>
        <ul class="subagents-rows subagents-branches"><For each={keys(group().rows)}>{id => <Row id={id} />}</For></ul>
      </details>
    </Show>}</Show>;
  };

  return (
    <div class="subagents-panel" classList={{ 'subagents-reading': !!openKey() }}>
      <div class="subagents-list" ref={list} hidden={!!openKey()}>
        <Show when={view()} fallback={<p class="subagents-empty" role="status">{read.error() ? `Couldn't read the subagents: ${read.error()}` : 'Reading subagents…'}</p>}>
          {v => <>
            <Show when={read.error()}><p class="subagents-error" role="status">Showing the last read. Couldn't refresh: {read.error()}</p></Show>
            <section class="subagents-group" aria-labelledby="subagents-active">
              <h3 id="subagents-active" class="subagents-group-head">Active · {v().running + v().queued}</h3>
              <Show when={v().active.length} fallback={<p class="subagents-empty">No active subagents</p>}>
                <For each={groups().filter(g => g.active).map(g => g.id)}>{id => <Group id={id} />}</For>
              </Show>
            </section>
            <details class="subagents-group subagents-done">
              <summary class="subagents-group-head">{v().doneCount} done</summary>
              <Show when={groups().some(g => g.active && g.rows.some(r => !live(r)))}><p class="subagents-empty">Finished parts stay with their active team above.</p></Show>
              <Show when={!v().done.length}><p class="subagents-empty">No finished subagents</p></Show>
              <For each={finishedGroups().map(g => g.id)}>{id => <Group id={id} />}</For>
              <Show when={v().doneOlder.length}>
                <h4 class="subagents-older-head">Older · {v().doneOlder.length}</h4>
                <ul class="subagents-rows"><For each={keys(v().doneOlder)}>{id => <Row id={id} />}</For></ul>
              </Show>
              <Show when={olderLeft(v())}>
                <button type="button" class="subagents-show-older" onClick={() => void read.showOlder()}>
                  Show older ({olderLeft(v())})
                </button>
              </Show>
            </details>
            <Show when={v().earlier.length}>
              <section class="subagents-group" aria-labelledby="subagents-earlier">
                <h3 id="subagents-earlier" class="subagents-group-head">Earlier chats · {v().earlier.length}</h3>
                <ul class="subagents-rows"><For each={keys(v().earlier)}>{id => <Row id={id} />}</For></ul>
              </section>
            </Show>
          </>}
        </Show>
      </div>
      <Show when={openKey()} keyed>
        {key => <SubagentSession openKey={key} row={() => findRow(view(), key)} at={at} now={now} names={names} onBack={back} onChanged={() => void read.refresh()} />}
      </Show>
    </div>
  );
}

/** Useful and Not useful under a transcript reply (overnight 11-12, C10): the
 *  chat's thumbs (`TurnFooter.tsx`), saved against the subagent's run id in
 *  the same `turn_feedback` store; Not useful then asks for an optional
 *  one-line reason. */
function ReplyFeedback(props: { reply: { answer: string; runId: string | null }; settled: boolean }) {
  const runId = () => props.reply.runId ?? undefined;
  const can = () => canVote({ role: 'assistant', text: props.reply.answer, runId: runId() }, null) && props.settled;
  const vote = () => (runId() ? replyVotes()[runId()!] : undefined);
  createEffect(() => { if (can()) ensureReplyVote(runId()); });
  const [reasonOpen, setReasonOpen] = createSignal(false);
  const [draft, setDraft] = createSignal('');
  async function cast(choice: FeedbackVote, reason?: string) {
    const id = runId();
    if (!id) return;
    try {
      await saveReplyVote(id, choice, reason);
      flashFooter(voteSavedText(choice, !!reason?.trim()), 1600);
      setReasonOpen(choice === 'notUseful' && reason === undefined);
      if (choice === 'useful' || reason !== undefined) setDraft('');
    } catch (e) { flashFooter(`× ${voteErrorText(e)}`, 4000); }
  }
  return (
    <Show when={can()}>
      <button type="button" class="subagents-foot-btn" classList={{ active: vote() === 'useful' }} aria-pressed={vote() === 'useful'} title="Useful" aria-label="Useful" onClick={() => void cast('useful')}><Icon name="thumbsUp" size={13} /></button>
      <button type="button" class="subagents-foot-btn" classList={{ active: vote() === 'notUseful' }} aria-pressed={vote() === 'notUseful'} title="Not useful" aria-label="Not useful" onClick={() => void cast('notUseful')}><Icon name="thumbsDown" size={13} /></button>
      <Show when={reasonOpen() && vote() === 'notUseful'}>
        <span class="subagents-feedback-reason">
          <input type="text" aria-label="What was wrong? (optional)" placeholder="What was wrong? (optional)" maxLength={2000} value={draft()}
            ref={el => requestAnimationFrame(() => el.focus({ preventScroll: true }))}
            onInput={e => setDraft(e.currentTarget.value)}
            onKeyDown={e => {
              e.stopPropagation();
              if (e.key === 'Enter' && draft().trim()) { e.preventDefault(); void cast('notUseful', draft()); }
              else if (e.key === 'Escape') { e.preventDefault(); setReasonOpen(false); }
            }} />
          <button type="button" class="btn ghost sm" disabled={!draft().trim()} onClick={() => void cast('notUseful', draft())}>Save</button>
        </span>
      </Show>
    </Show>
  );
}

function SubagentSession(props: { openKey: string; row: () => SubagentRow | null; at: () => string; now: () => number; names: () => Record<string, ModelNames>; onBack: () => void; onChanged: () => void }) {
  let backButton: HTMLButtonElement | undefined;
  queueMicrotask(() => backButton?.focus({ preventScroll: true }));
  // A chain that handed over reads its current conversation.
  const key = () => props.row()?.current ?? props.openKey;
  const [convo, setConvo] = createSignal<AgentConversation | null>(null);
  const [readError, setReadError] = createSignal<string | null>(null);
  const [bump, setBump] = createSignal(0);
  // Read again when its state, tokens or end change (a turn ended), or after a send.
  const stamp = createMemo(() => { const r = props.row(); return `${key()}|${r?.status}|${r?.tokens}|${r?.endedAt}|${bump()}`; });
  createEffect(on(stamp, () => {
    let current = true;
    const k = key();
    void api.readAgentConversation(k).then(c => { if (current) { setConvo(c); setReadError(null); } }, e => { if (current) setReadError(errMsg(e)); });
    onCleanup(() => { current = false; });
  }));
  const title = () => titleOf(props.row(), convo()?.summary.title);
  const agent = () => props.row()?.agent ?? convo()?.summary.agent ?? '';
  // A memo, not a plain function: replies() builds new objects on every call,
  // so `reply === list()[last]` (latest row, settled votes) never matched.
  const list = createMemo(() => replies(convo()?.turns ?? []));
  // The CFO's choice for the latest turn that recorded one.
  const cfo = () => cfoHeaderLine(latestCfo(list()), props.names()[agent()]);
  // Read aloud (`store/readAloud.ts`): one reply at a time; leaving the
  // transcript stops a read it started.
  const reader = {};
  const readKey = (r: { id: string; runId: string | null }) => `subagent:${r.runId ?? r.id}`;
  const toggleReading = (r: { id: string; runId: string | null; answer: string }) => {
    if (readingKey() === readKey(r)) stopReading();
    else readAloud(readKey(r), r.answer, reader);
  };
  onCleanup(() => stopReading(reader));
  // A handover moves the transcript to its successor chat: a read of the
  // last one stops with it (overnight 11-13).
  createEffect(on(key, () => stopReading(reader), { defer: true }));
  // Its turn is still running, so the answer is still streaming: no Copy,
  // Read aloud or vote until it ends (the turn's own status, overnight 11-13).
  // A turn with no recorded status (older builds) falls back to the last
  // reply of a live row that has no finish time yet.
  const streaming = (r: { running: boolean; finishedAt: string | null }) => {
    if (!live(props.row())) return false;
    const all = list();
    return r.running || (all[all.length - 1] === r && !r.finishedAt);
  };
  const copy = async (text: string) => {
    try { await navigator.clipboard.writeText(text); flashFooter('Copied reply', 1500); }
    catch (e) { flashFooter(`× Couldn't copy: ${errMsg(e)}`, 4000); }
  };
  // By its queue row, read before the confirm opens (`stopSubagent`): a stop
  // by key could end the next turn of the same chat.
  const stop = async () => {
    const name = title();
    const confirm = () => showConfirm({
      title: `Stop “${name}”?`,
      body: 'Its turn ends now. The chat that started it sees it was stopped the next time it runs.',
      confirmLabel: 'Stop', danger: true, focusCancel: true,
    });
    try {
      const done = await stopSubagent(props.row, { confirm, cancelRow: api.cancelQueuedRun });
      if (done === 'declined') return;
      flashFooter(done === 'noRow' ? NO_ROW_NOTICE : done === 'ended' ? `“${name}” had already ended, so nothing was stopped` : `Stopping “${name}”`, done === 'noRow' ? 5000 : undefined);
    }
    catch (e) { flashFooter(`× Couldn't stop it: ${errMsg(e)}`, 5000); }
    props.onChanged();
  };
  const [draft, setDraft] = createSignal('');
  const [sending, setSending] = createSignal(false);
  const [note, setNote] = createSignal('');
  const send = async () => {
    const text = draft().trim();
    if (!text || !convo() || sending()) return;
    setSending(true);
    // A handover moves the row to its current conversation before the read
    // of it lands: read that one first, so the message goes where it works.
    let c = convo()!;
    if (c.summary.key !== key()) {
      try { c = await api.readAgentConversation(key()); setConvo(c); }
      catch (e) { setNote(`Couldn't send: ${errMsg(e)}`); setSending(false); return; }
    }
    const s = c.summary;
    const permission = conversationPermission(s.key, c.permissionMode, s.agent);
    const native = permission.resumeNative && s.resumable === 'native' ? s.harnessSessionId : null;
    try {
      // Queued behind a running turn rather than refused (`after: 'queue'`).
      const accepted = await api.chatSend(text, native, s.cwd, props.row()?.model ?? undefined, permission.mode, s.agent, s.key, newClientTurnId(), false, null, [], null, 'queue');
      setDraft('');
      setNote(sentWords(accepted));
      setBump(n => n + 1);
      props.onChanged();
    } catch (e) { setNote(`Couldn't send: ${errMsg(e)}`); }
    finally { setSending(false); }
  };
  const keyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Enter' || e.isComposing) return;
    const sends = enterSends() ? !e.shiftKey && !e.metaKey : e.metaKey;
    if (sends) { e.preventDefault(); void send(); }
  };
  return (
    <div class="subagents-session" aria-label={`Subagent ${title()}`} role="region">
      <header class="subagents-session-head">
        <div class="subagents-session-bar">
          <button type="button" ref={backButton} class="chat-panel-iconbtn" aria-label="Back to subagents" title="Back to subagents" onClick={props.onBack}><Icon name="arrowLeft" size={14} /></button>
          <Show when={agent()}><AgentMark agent={agent()} size={18} /></Show>
          <h3 class="subagents-session-title" title={title()}>{title()}</h3>
          <Show when={props.row()?.trackerIssue}>{issue => <span class="subagents-issue">{issue()}</span>}</Show>
          <Show when={props.row()?.status === 'running'}><span class="sr-only">Running</span></Show>
          <Show when={props.row()}>{r => <span class="subagents-session-meta" title={rowTitle(r(), props.at(), props.now())}>
            <Show when={r().status === 'running'}><span class="live-dot on" aria-hidden="true" /></Show>
            {[...fresh(r()), headerMeta(r(), props.at(), props.now(), props.names()[r().agent]), costWords(r().costUsd)].join(' · ')}
          </span>}</Show>
          {/* A queued one has no turn to stop yet. */}
          <Show when={props.row()?.status === 'running'}><button type="button" class="btn ghost sm" onClick={() => void stop()}>Stop</button></Show>
        </div>
        <Show when={cfo()}>{line => <p class="subagents-session-cfo" title={line()}>{line()}</p>}</Show>
      </header>
      <div class="subagents-transcript">
        <Show when={convo()} fallback={<p class="subagents-empty" role="status">{readError() ? `Couldn't read this conversation: ${readError()}` : 'Opening conversation…'}</p>}>
          <Show when={list().length} fallback={<p class="subagents-empty">{live(props.row()) ? 'Waiting for its first turn.' : 'Nothing recorded yet.'}</p>}>
            <For each={list()}>{reply => (
              <article class="subagents-reply">
                <Show when={reply.ask}><div class="subagents-ask"><MarkdownView text={reply.ask!} class="subagents-md" onLinkClick={openLink} forbidMedia /></div></Show>
                <Show when={reply.workedMs != null}><p class="subagents-worked">Worked for {durationWords(reply.workedMs!)}</p></Show>
                <Show when={reply.answer}><MarkdownView text={reply.answer} class="subagents-md" onLinkClick={openLink} forbidMedia /></Show>
                <Show when={reply.error}><p class="subagents-error">{reply.error}</p></Show>
                <Show when={reply.answer || reply.historyRunId}>
                  <div class="subagents-reply-foot" role="group" aria-label="Reply details" classList={{ latest: reply === list()[list().length - 1], reading: readingKey() === readKey(reply), voted: !!reply.runId && !!replyVotes()[reply.runId] }}>
                    <Show when={reply.answer && !streaming(reply)}>
                      <span class="subagents-foot-actions">
                        <button type="button" class="subagents-foot-btn" title="Copy reply" aria-label="Copy reply" onClick={() => void copy(reply.answer)}><Icon name="copy" size={13} /></button>
                        <ReplyFeedback reply={reply} settled={!streaming(reply)} />
                        <Show when={readAloudSupported()}>
                          <button type="button" class="subagents-foot-btn" classList={{ active: readingKey() === readKey(reply) }} title={readingKey() === readKey(reply) ? 'Stop reading' : 'Read aloud'} aria-label={readingKey() === readKey(reply) ? 'Stop reading' : 'Read aloud'} aria-pressed={readingKey() === readKey(reply)}
                            onClick={() => toggleReading(reply)}>
                            <Show when={readingKey() === readKey(reply)} fallback={<SpeakerIcon />}><Icon name="stop" size={13} /></Show>
                          </button>
                        </Show>
                      </span>
                    </Show>
                    <Show when={finishedText(reply.finishedAt ?? undefined)}>{time => <span class="subagents-reply-time" title={new Date(reply.finishedAt!).toLocaleString()}>{time()}</span>}</Show>
                    <Show when={reply.historyRunId}>{run => <button type="button" class="btn-link subagents-history" onClick={() => openVaultHistory(undefined, run())}>Show change in history</button>}</Show>
                  </div>
                </Show>
              </article>
            )}</For>
            <Show when={props.row()?.status === 'running'}><p class="subagents-worked" role="status"><span class="live-dot on" aria-hidden="true" /> Working</p></Show>
          </Show>
        </Show>
      </div>
      {/* Overnight 11-12 (C8): one rounded box as the chat's composer, the
          words on top and a round Send inside at its bottom right. */}
      <form class="subagents-composer" onSubmit={e => { e.preventDefault(); void send(); }}>
        <div class="subagents-composer-box">
          <textarea class="subagents-input" rows={2} aria-label="Message this subagent" placeholder={live(props.row()) ? 'Message it; it goes when its turn ends' : 'Message this subagent'}
            value={draft()} onInput={e => setDraft(e.currentTarget.value)} onKeyDown={keyDown} disabled={!convo()} />
          <div class="subagents-composer-row">
            <span class="subagents-note" role="status">{note()}</span>
            <button type="submit" class="btn primary subagents-send" aria-label="Send" title={enterSends() ? 'Send (Enter)' : 'Send (⌘Enter)'}
              disabled={!draft().trim() || !convo() || sending()}><Icon name="arrowUp" size={16} /></button>
          </div>
        </div>
      </form>
    </div>
  );
}
