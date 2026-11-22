// Resume at a chosen time (P1c): a paused chat's "Resume at..." control.
// The button opens a small panel with presets (in 1 hour, tonight 9 PM,
// tomorrow 8 AM), a custom date and time, and Clear time once one is set;
// the chosen time shows beside it ("Resumes tomorrow at 08:00"). A time
// already past resumes now through `onResumeNow` (the composer's Resume, so
// the turn is adopted on screen). Lazy: the composer and Running load it
// only for a paused chat.
import { For, Show, createSignal, createUniqueId } from 'solid-js';
import { parseLocalDateTime, resumeAtLabel, resumePresets, toLocalDateTime } from '../../chatResumeAt';
import { setChatResumeAt } from '../../store/chatPauseActions';

export default function ResumeAtControl(props: {
  sessionKey: string;
  resumeAt: number | null | undefined;
  /** Resume now (a time already past). */
  onResumeNow: () => void;
  /** The clock (ms), so the label follows a ticking view. */
  now?: number;
}) {
  const id = createUniqueId();
  const [open, setOpen] = createSignal(false);
  const [custom, setCustom] = createSignal('');
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const now = () => props.now ?? Date.now();

  const choose = async (at: number | null) => {
    setError(null);
    if (at !== null && at * 1000 <= Date.now()) {
      setOpen(false);
      props.onResumeNow();
      return;
    }
    setBusy(true);
    const failed = await setChatResumeAt(props.sessionKey, at);
    setBusy(false);
    if (failed) setError(failed);
    else setOpen(false);
  };
  const setCustomTime = () => {
    const at = parseLocalDateTime(custom());
    if (at === null) { setError('Pick a date and a time.'); return; }
    void choose(at);
  };
  const toggle = () => {
    if (!open()) setCustom(toLocalDateTime(props.resumeAt ?? Math.floor(Date.now() / 1000) + 3600));
    setError(null);
    setOpen(o => !o);
  };

  return (
    <span class="chat-resume-at">
      <Show when={props.resumeAt}>{at => <span class="chat-resume-at-time">{resumeAtLabel(at(), now())}</span>}</Show>
      <button type="button" class="btn ghost small chat-resume-at-toggle" aria-expanded={open()} aria-controls={`${id}-panel`} disabled={busy()} onClick={toggle}>Resume at…</button>
      <Show when={open()}>
        <span class="chat-resume-at-panel" id={`${id}-panel`} role="group" aria-label="Resume at">
          <For each={resumePresets(Date.now())}>{p => (
            <button type="button" class="btn small" disabled={busy()} onClick={() => void choose(p.at)}>{p.label}</button>
          )}</For>
          <label class="chat-resume-at-custom">
            <span>Custom time</span>
            <input type="datetime-local" value={custom()} onInput={e => setCustom(e.currentTarget.value)} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); setCustomTime(); } }} />
          </label>
          <button type="button" class="btn small" disabled={busy()} onClick={setCustomTime}>Set time</button>
          <Show when={props.resumeAt}>
            <button type="button" class="btn ghost small" disabled={busy()} onClick={() => void choose(null)}>Clear time</button>
          </Show>
          <Show when={error()}>{e => <span class="chat-resume-at-error" role="alert">{e()}</span>}</Show>
        </span>
      </Show>
    </span>
  );
}
