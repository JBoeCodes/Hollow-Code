import { createSignal, For, onMount, Show } from 'solid-js';
import { api } from '../api';
import type { CalendarSettings, CalendarSourceStatus } from '../agendaTypes';
import { Select } from '../components/Select';
import { ErrorText } from '../components/ui';
import { trackTask } from '../store';
import { SettingsSheet, openerRef } from './SettingsSheet';

/** Reads never request permission; choosing Mac or Allow calendar access does. */
export default function CalendarSource() {
  const [view, setView] = createSignal<CalendarSourceStatus>();
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  async function load(settings?: CalendarSettings, requestAccess = false) {
    if (busy()) return;
    setBusy(true); setError('');
    try {
      setView(await (settings ? trackTask('Save Agenda calendar', () => api.setCalendarSource(settings, requestAccess)) : api.calendarSourceStatus()));
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); }
  }
  onMount(() => void load());
  function toggle(id: string, included: boolean) {
    const v = view(); if (!v) return;
    const ids = v.settings.calendarIds ?? v.calendars.map(c => c.id);
    void load({ ...v.settings, calendarIds: included ? [...ids, id] : ids.filter(x => x !== id) });
  }
  // Settings PR 5b: one row (the source menu, a summary and Refresh); which
  // calendars to include opens in a sheet.
  const opener = openerRef('[data-sheet-opener="calendar-pick"]');
  const [pick, setPick] = createSignal(false);
  const summary = (v: CalendarSourceStatus) => v.settings.calendarIds === null ? 'All calendars' : `${v.settings.calendarIds.length} of ${v.calendars.length} calendars`;
  return <section class="cx-block cx-calendar" aria-label="Agenda calendar settings" tabIndex={-1}>
    <div class="cx-block-head"><div><h3>Agenda calendar</h3><p>Read meetings for meeting prep and for agents. Hollow never changes calendar events.</p></div>
      <div class="cx-calendar-row">
        <Show when={view()}>{v =>
          <>
            <Select label="Agenda calendar" value={v().settingsValid === false ? '' : v().settings.source} disabled={busy()} options={[
              ...(v().settingsValid === false ? [{ value: '', label: 'Choose a calendar source', disabled: true }] : []),
              { value: 'mac', label: 'Calendars on this Mac' }, { value: 'microsoft', label: 'Microsoft 365' }, { value: 'off', label: 'Off' },
            ]} onChange={source => void load({ ...v().settings, source: source as CalendarSettings['source'] }, source === 'mac')}/>
            <Show when={v().settings.source === 'mac' && v().permission === 'granted'}>
              <span class="cx-calendar-summary">{summary(v())}</span>
              <button type="button" class="btn small" data-sheet-opener="calendar-pick" onClick={e => { opener.mark(e); setPick(true); }}>Choose calendars…</button>
            </Show>
          </>
        }</Show>
        <button class="btn small ghost" disabled={busy()} onClick={() => void load()}>Refresh calendars</button>
      </div></div>
    <Show when={error()}>{e => <ErrorText class="work-error" text={e()}/>}</Show>
    <Show when={view()} fallback={<p role="status">{error() ? 'Calendar settings could not load. Use Refresh calendars to retry.' : 'Loading calendar settings…'}</p>}>{v => <>
      <Show when={v().error}>{e => <ErrorText class="work-error" text={e()}/>}</Show>
      <Show when={v().settings.source === 'mac'}>
        <Show when={v().permission !== 'granted'}>
          <div class="cx-banner cx-calendar-access" role="status"><span>Calendar access is {['notDetermined', 'writeOnly'].includes(v().permission) ? 'not granted for reading yet' : v().permission}. Allow Hollow to read calendars, then refresh here.</span>
          <Show when={['notDetermined', 'writeOnly'].includes(v().permission)}><button class="btn small" disabled={busy()} onClick={() => void load(v().settings, true)}>Allow calendar access</button></Show>
          <button class="btn small" onClick={() => void api.openCalendarSettings().catch(e => setError(String(e)))}>Open System Settings</button></div>
        </Show>
        <Show when={pick() && v().permission === 'granted'}>
          <SettingsSheet id="settings-sheet-calendars" title="Calendars to include" returnTo={opener.get} onClose={() => setPick(false)}>
            <fieldset class="cx-options cx-calendar-list" disabled={busy()}><legend>Calendars to include</legend>
            <label class="cx-check"><input type="checkbox" checked={v().settings.calendarIds === null} onChange={e => void load({ ...v().settings, calendarIds: e.currentTarget.checked ? null : [] })}/>All calendars, including new ones</label>
            <For each={v().calendars} fallback={<p>No calendars found. Add an account in macOS Calendar, then refresh here.</p>}>{c => <label class="cx-check"><input type="checkbox" checked={v().settings.calendarIds === null || v().settings.calendarIds!.includes(c.id)} onChange={e => toggle(c.id, e.currentTarget.checked)}/>{c.title}{c.account ? ` · ${c.account}` : ''}</label>}</For>
            <For each={v().settings.calendarIds?.filter(id => !v().calendars.some(c => c.id === id))}>{(id, i) => <div class="cx-option"><span>Unavailable calendar {i() + 1}</span><button type="button" class="btn small" onClick={() => toggle(id, false)}>Remove unavailable calendar {i() + 1}</button></div>}</For>
          </fieldset>
          </SettingsSheet>
        </Show>
      </Show>
      <Show when={v().settings.source === 'microsoft'}><p>Uses Hollow’s Microsoft 365 sign-in below.</p></Show>
      <Show when={v().settingsValid !== false && v().settings.source === 'off'}><p role="status">Calendar is off. Tasks and local focus blocks stay available.</p></Show>
    </>}</Show>
  </section>;
}
