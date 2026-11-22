import { For, Show, createMemo, createResource, createSignal, createEffect, on } from 'solid-js';
import { clearRecordingFocus, recordingFocusRequest } from '../store/originFocus';
import { api } from '../api';
import { Icon } from '../icons';
import { Button, StatePanel, StatusPill, Skeleton, ErrorText, PlainErrorText } from './ui';
import { Select } from './Select';
import { NoteRenderer } from './NoteRenderer';
import { extractProposals, type TaskProposal } from '../recordingTasks';
import { freshness } from '../agenda';
import { flashFooter, openSettingsFromOutside, refetchTasks } from '../store';
import { projectWorkspaces, selectWorkspace } from '../store/workspaces';
import type { RecordingDetail, RecordingRow } from '../recordingTypes';
import { errMsg } from '../utils';
import { latestUnlessErrored } from '../stableResource';
import { RECORDINGS_SIGN_IN_ROW } from '../meetingsCopy';
import { transcriptMatchParts } from '../recordingSearch';
import '../styles/recordings.css';

/**
 * Library → Recordings (next-level P11): the user's Plaud recordings, read
 * on demand through Hollow's Plaud sign-in. Recordings stay in Plaud; the
 * reader shows the summary, a timecoded transcript and audio availability,
 * says plainly when one isn't available, links a recording to a workspace or
 * meeting only when the user picks one, keeps a local transcript copy only on
 * request, and turns listed action items into reviewable, idempotent task
 * proposals (unknown owners and dates stay unknown).
 */
export function RecordingsView() {
  const [days, setDays] = createSignal('14');
  // Opening reuses Plaud's list for two minutes; Refresh and Retry ask Plaud again.
  let fresh = false;
  const [list, { refetch: reread }] = createResource(days, d => { const f = fresh; fresh = false; return api.listRecordings(Number(d), f); });
  const refetch = () => { fresh = true; return reread(); };
  const [selected, setSelected] = createSignal<string | null>(null);
  // Read through latestUnlessErrored: `list.latest` throws once the listing
  // failed (a damaged or unreadable recording-links file, B·F08), which
  // would leave the pane on its loading skeleton and hide the error.
  const loaded = () => latestUnlessErrored(list);
  const rows = () => loaded()?.recordings ?? [];
  const current = () => selected() ?? rows()[0]?.id ?? null;
  /** The list's one Tab stop: the selected row, or the first when the
   *  selection isn't listed in this range (plan completion L5). */
  const tabStop = () => rows().some(r => r.id === current()) ? current() : rows()[0]?.id ?? null;
  // A task's "Open the recording" (plan completion item 4): select it once
  // listed, widening the range once to 90 days, or say it isn't listed.
  const [focusNote, setFocusNote] = createSignal('');
  createEffect(on([selected, days], () => setFocusNote(''), { defer: true }));
  createEffect(on(() => [recordingFocusRequest(), loaded()] as const, ([id, l]) => {
    if (!id || !l || list.loading) return;
    if (l.recordings.some(r => r.id === id)) { clearRecordingFocus(); setSelected(id); setFocusNote(''); return; }
    if (l.state === 'ok' && days() !== '90') { setDays('90'); return; }
    clearRecordingFocus();
    setFocusNote(l.state === 'ok' ? 'The recording that task came from isn’t listed in the last 90 days.' : 'The recording that task came from can’t be shown while recordings are unavailable.');
  }));
  return <div class="recordings">
    <aside class="recordings-list" aria-label="Recordings">
      <header class="recordings-head">
        <div class="recordings-source">
          <Show when={loaded()} fallback={list.error ? <StatusPill quiet label="Recordings unavailable" tone="error" /> : <StatusPill quiet label="Loading" tone="busy" />}>{l =>
            <Show when={l().state === 'ok'} fallback={<StatusPill quiet label={l().state === 'notConnected' ? 'Plaud not connected' : 'Plaud unavailable'} tone={l().state === 'notConnected' ? 'off' : 'error'} hint={l().error ?? undefined} />}>
              <StatusPill quiet label={`${l().source} · ${freshness(l().fetchedAt)}`} tone="ok" />
            </Show>}
          </Show>
        </div>
        <div class="recordings-tools">
          <Select size="sm" label="Recordings from" value={days()} searchable={false} onChange={setDays} options={[{ value: '7', label: 'Last 7 days' }, { value: '14', label: 'Last 14 days' }, { value: '30', label: 'Last 30 days' }, { value: '90', label: 'Last 90 days' }]} />
          <button class="icon-btn" aria-label="Refresh recordings" title="Refresh" onClick={() => void refetch()}><Icon name="refresh" size={14} /></button>
        </div>
      </header>
      <Show when={focusNote()}><p class="work-muted recordings-focus-note" role="status">{focusNote()}</p></Show>
      <Show when={loaded()} fallback={<Show when={list.error} fallback={<Skeleton rows={5} label="Loading recordings" />}><StatePanel compact kind="failed" title="Recordings couldn’t be listed" actions={<Button size="sm" onClick={() => void refetch()}>Retry</Button>} error={list.error} /></Show>}>{l =>
        <Show when={l().state === 'ok'} fallback={
          <StatePanel compact kind={l().state === 'notConnected' ? 'blocked' : 'offline'} title={l().state === 'notConnected' ? 'Plaud isn’t connected' : 'Plaud couldn’t be reached'} actions={l().state === 'notConnected'
            // Lands on Settings' Recordings in Hollow row and focuses its Connect (P1-20).
            ? <Button size="sm" onClick={() => openSettingsFromOutside('connectors', RECORDINGS_SIGN_IN_ROW)}>Sign in to Plaud for Recordings</Button>
            : <Button size="sm" onClick={() => openSettingsFromOutside('connectors')}>Open Connectors</Button>}>
            {l().state === 'notConnected' ? 'Sign in to Plaud for Recordings in Settings → Connectors to read your recordings here. Nothing is copied unless you choose Keep a transcript copy.' : l().error ? <PlainErrorText text={l().error!} /> : 'Try again in a moment.'}
          </StatePanel>}>
          {/* A listbox moves with the arrows (one Tab stop, the selected row), as
              Results' does; it was Tab-only (plan completion L5, keyboard pass). */}
          <Show when={rows().length} fallback={<StatePanel compact kind="empty" title="No recordings in this range">Try a longer range.</StatePanel>}>
          <ul class="recordings-rows" role="listbox" aria-label="Recordings" onKeyDown={e => {
            const step = e.key === 'ArrowDown' ? 1 : e.key === 'ArrowUp' ? -1 : 0;
            if (!step && e.key !== 'Home' && e.key !== 'End') return;
            const ids = rows().map(r => r.id);
            if (!ids.length) return;
            e.preventDefault();
            const at = ids.indexOf(current() ?? '');
            const next = e.key === 'Home' || at < 0 ? 0 : e.key === 'End' ? ids.length - 1 : Math.min(ids.length - 1, Math.max(0, at + step));
            const list = e.currentTarget;
            setSelected(ids[next]);
            const focusRow = () => list.querySelectorAll<HTMLButtonElement>('[role=option]')[next]?.focus();
            focusRow();
            // Again once the selection has re-rendered the rows.
            requestAnimationFrame(focusRow);
          }}>
            <For each={rows()}>{r =>
              <li role="presentation"><button type="button" role="option" class="recordings-row" aria-selected={current() === r.id} tabindex={tabStop() === r.id ? 0 : -1} onClick={() => setSelected(r.id)}>
                <span class="recordings-row-icon" aria-hidden="true"><Icon name="mic" size={14} /></span>
                <span class="recordings-row-main"><strong>{r.name || 'Untitled recording'}</strong><small>{r.date ?? 'No date'}{r.duration ? ` · ${r.duration}` : ''}</small></span>
                <Show when={l().links[r.id]?.workspaceId}>{wid => <span class="recordings-row-link" title="Linked to a workspace"><Icon name="link" size={11} />{projectWorkspaces().find(w => w.id === wid())?.title ?? 'Workspace'}</span>}</Show>
              </button></li>}
            </For>
          </ul>
          </Show>
          <Show when={l().unparsed}><p class="recordings-note">{l().unparsed} line{l().unparsed === 1 ? '' : 's'} from Plaud couldn’t be read and {l().unparsed === 1 ? 'is' : 'are'} not shown.</p></Show>
        </Show>}
      </Show>
    </aside>
    <section class="recordings-reader" aria-label="Recording">
      <Show when={current()} keyed fallback={<StatePanel kind="empty" title="Choose a recording">Its summary, transcript and proposed tasks show here.</StatePanel>}>
        {id => <RecordingReader id={id} row={rows().find(r => r.id === id) ?? null} onLinked={() => void reread()} />}
      </Show>
    </section>
  </div>;
}

function RecordingReader(props: { id: string; row: RecordingRow | null; onLinked: () => void }) {
  const [detail, { refetch }] = createResource(() => props.id, id => api.readRecording(id));
  const [filter, setFilter] = createSignal('');
  let filterInput: HTMLInputElement | undefined;
  const [chosen, setChosen] = createSignal<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  const [audio, setAudio] = createSignal<string | null>(null);
  const [created, setCreated] = createSignal<{ made: number; existing: number } | null>(null);
  const proposals = createMemo<TaskProposal[]>(() => { const d = detail.latest; return d ? extractProposals(d.id, d.summary, d.transcript) : []; });
  const segments = createMemo(() => { const q = filter().trim().toLowerCase(); const t = detail.latest?.transcript ?? []; return q ? t.filter(s => s.text.toLowerCase().includes(q) || (s.speaker ?? '').toLowerCase().includes(q)) : t; });
  const name = () => detail.latest?.name ?? props.row?.name ?? 'Recording';
  const toggle = (k: string) => setChosen(prev => { const n = new Set(prev); if (n.has(k)) n.delete(k); else n.add(k); return n; });
  async function link(workspaceId: string) {
    setError('');
    try { await api.setRecordingLink(props.id, { workspaceId: workspaceId || null, eventId: detail.latest?.link?.eventId ?? null, eventTitle: detail.latest?.link?.eventTitle ?? null, name: name() }); await refetch(); props.onLinked(); flashFooter(workspaceId ? 'Linked. The recording stays in Plaud.' : 'Link removed.'); }
    catch (e) { setError(errMsg(e)); }
  }
  async function createTasks(d: RecordingDetail) {
    const picked = proposals().filter(p => chosen().has(p.idempotencyKey));
    if (!picked.length || busy()) return;
    setBusy(true); setError(''); setCreated(null); let made = 0, existing = 0;
    const project = projectWorkspaces().find(w => w.id === d.link?.workspaceId)?.title;
    // A failure part-way keeps what was made on the record and leaves only the
    // rest chosen; creating again is safe (idempotency keys), and says so
    // (plan completion L5: the made ones went unreported before).
    const done = new Set<string>();
    try {
      for (const p of picked) {
        const r = await api.createTask({ title: p.text, due: p.due ?? undefined, description: [`From the recording “${name()}”${p.at ? ` at ${p.at}` : ''}.`, p.quote ? `> ${p.quote}` : '', p.owner ? `Owner named in the recording: ${p.owner}` : 'Owner: not stated in the recording.'].filter(Boolean).join('\n\n'), idempotencyKey: p.idempotencyKey, origin: p.origin, ...(project ? { project } : {}) });
        if (r.existing) existing++; else made++;
        done.add(p.idempotencyKey);
      }
      setChosen(new Set<string>());
    } catch (e) {
      const left = picked.filter(p => !done.has(p.idempotencyKey));
      setChosen(new Set(left.map(p => p.idempotencyKey)));
      setError(`${left.length === 1 ? `“${left[0].text}” wasn’t created` : `${left.length} tasks weren’t created`}: ${errMsg(e).replace(/\.$/, '')}. ${left.length === 1 ? 'It stays' : 'They stay'} chosen: Create again to retry. Tasks already made won’t be duplicated.`);
    } finally {
      if (made || existing || done.size === picked.length) setCreated({ made, existing });
      if (done.size) void refetchTasks();
      setBusy(false);
    }
  }
  async function keep(d: RecordingDetail) {
    const wid = d.link?.workspaceId; if (!wid) return;
    setError('');
    try { const path = await api.keepRecordingTranscript(props.id, wid); await refetch(); flashFooter(`Kept a copy in ${path}.`); } catch (e) { setError(errMsg(e)); }
  }
  return <Show when={detail.latest} fallback={<Show when={detail.error} fallback={<Skeleton rows={6} label="Opening the recording" />}><StatePanel kind="failed" title="This recording couldn’t be opened" actions={<Button size="sm" onClick={() => void refetch()}>Retry</Button>} error={detail.error} /></Show>}>{d =>
    <article class="recording-reader">
      <header class="recording-head">
        <span class="recording-kicker"><Icon name="mic" size={13} />Recording · Plaud · {freshness(d().fetchedAt)}</span>
        <h1>{name()}</h1>
        <p class="recording-meta">{props.row?.date ?? 'No date'}{props.row?.duration ? ` · ${props.row.duration}` : ''}</p>
        <div class="recording-states">
          <StatusPill quiet label={d().summaryState === 'available' ? 'Summary' : d().summaryState === 'unavailable' ? 'No summary' : 'Summary unavailable'} tone={d().summaryState === 'available' ? 'ok' : 'off'} />
          <StatusPill quiet label={d().transcriptState === 'available' ? `Transcript · ${d().transcript?.length ?? 0} segments` : d().transcriptState === 'unavailable' ? 'No transcript' : 'Transcript unavailable'} tone={d().transcriptState === 'available' ? 'ok' : 'off'} />
          <StatusPill quiet label={d().audio === 'available' ? 'Audio in Plaud' : d().audio === 'unavailable' ? 'No audio' : 'Audio unknown'} tone={d().audio === 'available' ? 'ok' : 'off'} />
          <Show when={d().audio === 'available'}><Button size="sm" variant="ghost" icon="play" onClick={() => void api.recordingAudioUrl(props.id).then(setAudio).catch(e => setError(errMsg(e)))}>{audio() ? 'Refresh audio link' : 'Get audio link'}</Button></Show>
        </div>
        <Show when={audio()}>{u => <p class="recording-audio"><audio controls preload="none" src={u()} aria-label={`Audio of ${name()}`} /><small>The link expires after about a day; get a new one any time.</small></p>}</Show>
        <div class="recording-link">
          <Show when={d().link?.eventId}>{event => <>
            <span>Linked meeting: {d().link?.eventTitle ?? 'Calendar meeting'}</span>
            <Button size="sm" variant="ghost" disabled={!d().link?.eventStart && !props.row?.date} onClick={() => {
              const date = d().link?.eventStart ?? props.row?.date;
              if (date) void import('../store/prepPack').then(m => m.openMeetingRecap(event(), d().link?.eventTitle ?? 'Calendar meeting', /^\d{4}-\d{2}-\d{2}$/.test(date) ? new Date(`${date}T12:00:00`).toISOString() : date));
            }}>Recap the meeting</Button>
            <Show when={!d().link?.eventStart && !props.row?.date}><small>Open this meeting in Agenda to recap it; the recording date is unavailable.</small></Show>
          </>}</Show>
          <span>Linked workspace</span>
          <Select size="sm" label="Link to a workspace" placeholder="Not linked: choose a workspace…" value={d().link?.workspaceId ?? ''} onChange={link} options={[...(d().link?.workspaceId ? [{ value: '', label: 'Remove the link' }] : []), ...projectWorkspaces().map(w => ({ value: w.id, label: w.title }))]} />
          <Show when={d().link?.workspaceId}>{wid => <>
            <Button size="sm" variant="ghost" onClick={() => void selectWorkspace(wid())}>Open workspace</Button>
            <Show when={!d().link?.keptCopy && d().transcriptState === 'available'} fallback={<Show when={d().link?.keptCopy}><small class="recording-kept">Copy kept: {d().link!.keptCopy}</small></Show>}>
              <Button size="sm" variant="ghost" title="Writes this transcript into the workspace’s sources, with its Plaud id. Optional; nothing is copied otherwise." onClick={() => void keep(d())}>Keep a transcript copy there</Button>
            </Show>
          </>}</Show>
        </div>
        <Show when={error()}>{msg => <ErrorText class="work-error" text={msg()}/>}</Show>
        <Show when={d().errors.length}><p class="recording-errors">Some parts couldn’t be read: {d().errors.join('; ')}</p></Show>
      </header>
      <div class="recording-body">
        <div class="recording-main">
          <section class="recording-section" aria-label="Summary">
            <h2>Summary</h2>
            <Show when={d().summary} fallback={<p class="recording-missing">{d().summaryState === 'error' ? 'The summary couldn’t be read from Plaud.' : 'Plaud has no summary for this recording.'}</p>}><div class="recording-summary"><NoteRenderer content={d().summary!} /></div></Show>
          </section>
          <section class="recording-section" aria-label="Transcript">
            <h2>Transcript <Show when={d().transcript}><input ref={filterInput} class="recording-filter" type="search" aria-label="Find in transcript" placeholder="Find in transcript" value={filter()} onInput={e => setFilter(e.currentTarget.value)} /></Show></h2>
            <Show when={d().transcript} fallback={<p class="recording-missing">{d().transcriptState === 'error' ? 'The transcript couldn’t be read from Plaud.' : 'Plaud has no transcript for this recording.'}</p>}>
              <div class="recording-search-status" classList={{ 'is-searching': !!filter().trim() }}><span role="status">{filter().trim() ? `${segments().length} matching segment${segments().length === 1 ? '' : 's'} of ${d().transcript!.length}` : ''}</span><Show when={filter().trim()}><Button size="sm" variant="ghost" onClick={() => { setFilter(''); filterInput?.focus(); }}>Clear search</Button></Show></div>
              <ol class="recording-transcript"><For each={segments()} fallback={<li class="recording-missing">No line matches.</li>}>{s => <li><time>{s.start}</time><div class="recording-segment"><Show when={s.speaker}><strong><TranscriptMatch text={s.speaker!} query={filter()} /></strong></Show><span class="recording-words"><TranscriptMatch text={s.text} query={filter()} /></span></div></li>}</For></ol>
            </Show>
          </section>
        </div>
        <aside class="recording-side" aria-label="Proposed tasks">
          <h2>Proposed tasks <span class="ui-section-count">{proposals().length}</span></h2>
          <p class="recording-side-note">From the summary’s action items. Owners and dates appear only when the recording states them. Nothing is created until you choose.</p>
          <ul class="recording-proposals"><For each={proposals()} fallback={<li class="recording-missing">No action items listed in the summary.</li>}>{p =>
            <li><label><input type="checkbox" checked={chosen().has(p.idempotencyKey)} onChange={() => toggle(p.idempotencyKey)} /><span><strong>{p.text}</strong>
              <small>{p.owner ? `Owner: ${p.owner}` : 'Owner unknown'} · {p.due ? `Due ${p.due}` : 'No date stated'} · {p.at ? `at ${p.at}` : 'no matching moment'}</small></span></label></li>}
          </For></ul>
          <Show when={proposals().length}><Button size="sm" variant="primary" disabled={!chosen().size || busy()} onClick={() => void createTasks(d())}>{busy() ? 'Creating…' : `Create ${chosen().size || ''} task${chosen().size === 1 ? '' : 's'}`.replace('  ', ' ')}</Button></Show>
          <Show when={created()}>{c => <p class="recording-created" role="status">{c().made} created{c().existing ? `, ${c().existing} already existed (not duplicated)` : ''}.</p>}</Show>
        </aside>
      </div>
    </article>}
  </Show>;
}

function TranscriptMatch(props: { text: string; query: string }) {
  const parts = createMemo(() => transcriptMatchParts(props.text, props.query));
  return <For each={parts()}>{part => part.match ? <mark>{part.text}</mark> : part.text}</For>;
}
