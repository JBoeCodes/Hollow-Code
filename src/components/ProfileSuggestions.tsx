/**
 * Library, Learnings: the profile builder's questions and suggestions, as
 * You items in the one review queue (Settings PR 4, overnight 11-12; they
 * were Settings, Personalization). The focus card, Ready to
 * add and Review all are the same controls as before, and an Add writes the
 * same `me/` note through the same `profile_builder_accept` call
 * (`store/profileBuilder.ts`). Scanning, the profile notes and Keep learning
 * stay in Settings, Personalization, which links here.
 */
import { For, Show, createMemo, createSignal } from 'solid-js';
import { api } from '../api';
import { groupSuggestions, sourceName, targetLabel } from '../aboutYou';
import { actProfile, acceptProfileSuggestion, profileBusy, profileLoadError, profileView, useProfileBuilder } from '../store/profileBuilder';
import { Icon } from '../icons';
import { ErrorText } from './ui';
import '../styles/about-you.css';

export function ProfileSuggestions() {
  useProfileBuilder();
  const view = profileView;
  const [editing, setEditing] = createSignal<string | null>(null);
  const [editText, setEditText] = createSignal('');
  const [answers, setAnswers] = createSignal<Record<string, string>>({});
  const order = createMemo(() => (view()?.profile ?? []).map(p => p.path.replace(/^me\//, '')));
  const groups = createMemo(() => groupSuggestions(view()?.suggestions ?? [], order()));
  // One suggestion at a time (audit H15): a focused queue instead of a wall
  // of Add/Edit/Dismiss rows. Skip only reorders locally; nothing is saved.
  const [skipped, setSkipped] = createSignal<string[]>([]);
  const queue = createMemo(() => { const all = groups().flatMap(g => g.items); const skip = skipped(); return [...all.filter(s => !skip.includes(s.id)), ...all.filter(s => skip.includes(s.id))]; });
  const current = () => queue()[0];
  // The focus card edits on its own, so the same suggestion open in the full
  // list never shows two edit boxes.
  const [focusEdit, setFocusEdit] = createSignal<string | null>(null);
  const [focusText, setFocusText] = createSignal('');

  function add(id: string, text?: string) {
    void acceptProfileSuggestion(id, text).then(() => {
      if (editing() === id) setEditing(null);
    });
  }

  function answer(id: string) {
    const reply = (answers()[id] ?? '').trim();
    if (!reply) return;
    void actProfile(id, () => api.profileBuilderAnswer(id, reply), 'Saved to your profile').then(() => {
      setAnswers(prev => { const next = { ...prev }; delete next[id]; return next; });
    });
  }

  return (
    <div class="about-you profile-suggestions" role="group" aria-label="From your profile">
      <Show when={profileLoadError()}>
        <ErrorText class="about-you-error" text={profileLoadError()} />
      </Show>
      <Show when={view()}>
        {view => (
          <>
      <Show when={view().questions.length || view().suggestions.length}>
        <p class="about-you-muted">You: what Hollow learned about you from your agents. Nothing goes into your profile until you add it, and an Add writes the same profile note as before.</p>
      </Show>
      <Show when={view().questions.length}>
        <section class="about-you-block" aria-label="Questions">
          <div class="about-you-block-head">
            <h3>A few things Hollow doesn’t know yet</h3>
          </div>
          <div class="about-you-questions">
            <For each={view().questions}>
              {q => (
                <div class="about-you-question" data-id={q.id}>
                  <label class="about-you-question-text" for={`aq-${q.id}`}>{q.question}</label>
                  <Show when={q.why}><div class="about-you-muted">{q.why}</div></Show>
                  <div class="about-you-question-row">
                    <input
                      id={`aq-${q.id}`}
                      class="settings-input"
                      placeholder={q.label}
                      value={answers()[q.id] ?? ''}
                      onInput={e => { const value = e.currentTarget.value; setAnswers(prev => ({ ...prev, [q.id]: value })); }}
                      onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); answer(q.id); } }}
                    />
                    <button class="btn" disabled={!!profileBusy() || !(answers()[q.id] ?? '').trim()} onClick={() => answer(q.id)}>Save</button>
                    <button class="btn ghost" disabled={!!profileBusy()} onClick={() => void actProfile(q.id, () => api.profileBuilderSkipQuestion(q.id))}>Don’t ask again</button>
                  </div>
                  <div class="about-you-target">Goes in {targetLabel(q)}</div>
                </div>
              )}
            </For>
          </div>
        </section>
      </Show>

      <Show when={view().suggestions.length}>
        <section class="about-you-block" aria-label="Ready to add">
          <div class="about-you-block-head">
            <h3>Ready to add</h3>
            <button class="btn" disabled={!!profileBusy()} onClick={() => void actProfile('all', () => api.profileBuilderAcceptAll(), 'Added to your profile')}>
              <Icon name="check" /> Add all {view().suggestions.length}
            </button>
          </div>
          <Show when={current()}>{s =>
            <div class="about-you-focus" aria-live="polite">
              <span class="about-you-focus-count">Suggestion 1 of {queue().length} · {groups().find(g => g.items.some(i => i.id === s().id))?.title}</span>
              <Show when={focusEdit() === s().id} fallback={<p class="about-you-focus-text">{s().label ? <strong>{s().label}: </strong> : null}{s().text}</p>}>
                <input class="settings-input" aria-label="Edit suggestion before adding" value={focusText()} onInput={e => setFocusText(e.currentTarget.value)}
                  onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); setFocusEdit(null); add(s().id, focusText()); } if (e.key === 'Escape') setFocusEdit(null); }} />
              </Show>
              <span class="about-you-target">from {sourceName(s().source)} · {targetLabel(s())}</span>
              <div class="about-you-focus-actions">
                <button class="btn primary" disabled={!!profileBusy()} onClick={() => { const edited = focusEdit() === s().id ? focusText() : undefined; setFocusEdit(null); add(s().id, edited); }}>Add to profile</button>
                <Show when={focusEdit() !== s().id}><button class="btn ghost" disabled={!!profileBusy()} onClick={() => { setFocusText(s().text); setFocusEdit(s().id); }}>Edit first</button></Show>
                <button class="btn ghost" disabled={!!profileBusy()} onClick={() => void actProfile(s().id, () => api.profileBuilderDismiss(s().id))}>Not right</button>
                <Show when={queue().length > 1}><button class="btn ghost" disabled={!!profileBusy()} onClick={() => { setFocusEdit(null); setSkipped(list => [...list.filter(x => x !== s().id), s().id]); }}>Skip for now</button></Show>
              </div>
            </div>
          }</Show>
          <details class="about-you-all">
            <summary>Review all {view().suggestions.length} at once</summary>
          <For each={groups()}>
            {g => (
              <div class="about-you-group">
                <div class="about-you-group-title">{g.title}</div>
                <For each={g.items}>
                  {s => (
                    <div class="about-you-suggestion" data-id={s.id}>
                      <div class="about-you-suggestion-copy">
                        <Show
                          when={editing() === s.id}
                          fallback={<div class="about-you-suggestion-text">{s.label ? <strong>{s.label}: </strong> : null}{s.text}</div>}
                        >
                          <input
                            class="settings-input"
                            aria-label="Edit before adding"
                            value={editText()}
                            onInput={e => setEditText(e.currentTarget.value)}
                            onKeyDown={e => {
                              if (e.key === 'Enter') { e.preventDefault(); add(s.id, editText()); }
                              if (e.key === 'Escape') setEditing(null);
                            }}
                          />
                        </Show>
                        <div class="about-you-target">from {sourceName(s.source)} · {targetLabel(s)}</div>
                      </div>
                      <div class="about-you-suggestion-actions">
                        <button
                          class="btn"
                          disabled={!!profileBusy()}
                          onClick={() => add(s.id, editing() === s.id ? editText() : undefined)}
                          aria-label={`Add to profile: ${s.text}`}
                        >Add</button>
                        <Show when={editing() !== s.id}>
                          <button class="btn ghost" title="Edit before adding" aria-label={`Edit before adding: ${s.text}`} disabled={!!profileBusy()} onClick={() => { setEditText(s.text); setEditing(s.id); }}>
                            <Icon name="edit" />
                          </button>
                        </Show>
                        <button class="btn ghost" title="Not right" aria-label={`Not right: ${s.text}`} disabled={!!profileBusy()} onClick={() => void actProfile(s.id, () => api.profileBuilderDismiss(s.id))}>
                          <Icon name="trash" />
                        </button>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            )}
          </For>
          </details>
        </section>
      </Show>
          </>
        )}
      </Show>
    </div>
  );
}
