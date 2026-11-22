import { For, Match, Show, Switch, createMemo } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { Icon } from '../icons';
import { MarkdownView } from './MarkdownView';
import { AS_MESSAGE_NOTE, approveAll, buildAnswer, cardTitle, formErrors, initialFormState, optionOutcome, questionLabel, questionReason, questionScenario, type QuestionFormState } from '../questionCards';
import type { ChatInteraction, InteractionAnswer, InteractionQuestion, InteractionVerdict } from '../types';
import '../styles/question-card.css';

const VERDICT_LABELS: Record<InteractionVerdict, string> = { approve: 'Approve', reject: 'Reject', defer: 'Defer' };

/**
 * A structured question card (Paperclip PR 13). Agent-authored text renders
 * as plain text; only `detailsMarkdown` and `previewMarkdown` go through
 * MarkdownView (marked + DOMPurify). The backend validates again on Submit.
 */
export function QuestionCard(props: {
  card: ChatInteraction;
  /** Why Submit is disabled right now (busy, loading, stale), or null. Shown
   *  above the questions, before any answer is picked. */
  blockReason: string | null;
  /** The answers go as an ordinary message (the session that asked can't be
   *  resumed); said above the questions. */
  asMessage?: boolean;
  submitting: boolean;
  error: string | null;
  onSubmit: (answer: InteractionAnswer) => void;
  onDismiss: () => void;
}) {
  const request = () => props.card.request;
  const [form, setForm] = createStore<QuestionFormState>(initialFormState(props.card.request));
  const errors = createMemo(() => formErrors(request(), form));
  const stale = () => props.card.status === 'stale';
  const canSubmit = () => !props.blockReason && !props.submitting && Object.keys(errors()).length === 0;
  const idFor = (...parts: string[]) => `qc-${props.card.id}-${parts.join('-')}`;
  // `:` cannot occur in a question or option id, so these never match an idFor id.
  const outcomeId = (q: InteractionQuestion, optionId: string) => `qc-${props.card.id}-outcome:${q.id}:${optionId}`;
  const scenarioId = (q: InteractionQuestion) => `qc-${props.card.id}-scenario:${q.id}`;

  function toggleChoice(q: InteractionQuestion, optionId: string) {
    setForm(produce(state => {
      const a = state.answers[q.id];
      if (!a) return;
      if (q.answerMode === 'single') {
        a.selected = [optionId];
        a.otherOn = false;
      } else if (a.selected.includes(optionId)) {
        a.selected = a.selected.filter(id => id !== optionId);
      } else {
        a.selected = [...a.selected, optionId];
      }
    }));
  }

  function toggleOther(q: InteractionQuestion) {
    setForm(produce(state => {
      const a = state.answers[q.id];
      if (!a) return;
      a.otherOn = q.answerMode === 'single' ? true : !a.otherOn;
      if (q.answerMode === 'single') a.selected = [];
    }));
  }

  function submit(e: Event) {
    e.preventDefault();
    if (!canSubmit()) return;
    props.onSubmit(buildAnswer(request(), form));
  }

  return (
    <form
      class="question-card"
      classList={{ 'is-stale': stale() }}
      role="group"
      aria-label={cardTitle(props.card)}
      data-interaction-id={props.card.id}
      onSubmit={submit}
    >
      <div class="question-card-head">
        <Icon name="message" size={14} />
        <span class="question-card-kicker">{stale() ? 'Out of date' : 'The agent is asking'}</span>
        <span class="question-card-title">{cardTitle(props.card)}</span>
      </div>
      <Show when={request().description}>
        <p class="question-card-desc">{request().description}</p>
      </Show>
      <Show when={props.blockReason}>
        <p class="question-card-block" id={idFor('block')}>{props.blockReason}</p>
      </Show>
      <Show when={props.asMessage && !props.blockReason}>
        <p class="question-card-desc question-card-as-message" id={idFor('as-message')} role="note">{AS_MESSAGE_NOTE}</p>
      </Show>

      <Switch>
        <Match when={request().mode === 'questions'}>
          <For each={request().questions ?? []}>
            {(q) => (
              <fieldset class="question-card-q" disabled={stale()} aria-describedby={questionScenario(q) ? scenarioId(q) : undefined}>
                <legend class="question-card-prompt">
                  {q.prompt}
                  <Show when={q.required === false}><span class="question-card-optional"> (optional)</span></Show>
                </legend>
                <Show when={questionReason(q)}><p class="question-card-help">{questionReason(q)}</p></Show>
                <Show when={questionScenario(q)}><p class="question-card-scenario" id={scenarioId(q)}>{questionScenario(q)}</p></Show>
                <Show
                  when={q.answerMode !== 'text'}
                  fallback={
                    <textarea
                      class="question-card-text"
                      rows={2}
                      aria-label={questionLabel(q)}
                      value={form.answers[q.id]?.text ?? ''}
                      onInput={e => setForm('answers', q.id, 'text', e.currentTarget.value)}
                    />
                  }
                >
                  <div class="question-card-options">
                    <For each={q.options ?? []}>
                      {(option) => (
                        <label class="question-card-option" for={idFor(q.id, option.id)}>
                          <input
                            id={idFor(q.id, option.id)}
                            type={q.answerMode === 'single' ? 'radio' : 'checkbox'}
                            name={idFor(q.id)}
                            checked={(form.answers[q.id]?.selected ?? []).includes(option.id)}
                            aria-describedby={optionOutcome(option) ? outcomeId(q, option.id) : undefined}
                            onChange={() => toggleChoice(q, option.id)}
                          />
                          <span class="question-card-option-label">{option.label}</span>
                          <Show when={option.recommended}><span class="question-card-badge">Recommended</span></Show>
                          <Show when={option.description}><span class="question-card-option-desc">{option.description}</span></Show>
                          <Show when={optionOutcome(option)}><span class="question-card-option-desc question-card-outcome" id={outcomeId(q, option.id)} aria-hidden="true">{optionOutcome(option)}</span></Show>
                        </label>
                      )}
                    </For>
                    <Show when={q.allowCustom}>
                      <label class="question-card-option" for={idFor(q.id, 'other')}>
                        <input
                          id={idFor(q.id, 'other')}
                          type={q.answerMode === 'single' ? 'radio' : 'checkbox'}
                          name={idFor(q.id)}
                          checked={form.answers[q.id]?.otherOn ?? false}
                          onChange={() => toggleOther(q)}
                        />
                        <span class="question-card-option-label">{q.customLabel || 'Other…'}</span>
                      </label>
                      <Show when={form.answers[q.id]?.otherOn}>
                        <input
                          class="question-card-other"
                          type="text"
                          aria-label={`${questionLabel(q)}: your answer`}
                          value={form.answers[q.id]?.other ?? ''}
                          onInput={e => setForm('answers', q.id, 'other', e.currentTarget.value)}
                        />
                      </Show>
                    </Show>
                  </div>
                </Show>
                <Show when={errors()[q.id]}><p class="question-card-field-error">{errors()[q.id]}</p></Show>
              </fieldset>
            )}
          </For>
        </Match>

        <Match when={request().mode === 'confirmation' && request().confirmation}>
          {(c) => (
            <fieldset class="question-card-q" disabled={stale()}>
              <legend class="question-card-prompt">{c().prompt}</legend>
              <Show when={c().detailsMarkdown}><MarkdownView text={c().detailsMarkdown!} class="question-card-md" /></Show>
              <Show when={c().target}><p class="question-card-target"><code>{c().target!.path}</code></p></Show>
              <div class="question-card-choices">
                <button type="button" class="question-card-choice" classList={{ on: form.accepted === true }} aria-pressed={form.accepted === true} onClick={() => setForm('accepted', true)}>
                  {c().acceptLabel || 'Accept'}
                </button>
                <button type="button" class="question-card-choice" classList={{ on: form.accepted === false }} aria-pressed={form.accepted === false} onClick={() => setForm('accepted', false)}>
                  {c().rejectLabel || 'Reject'}
                </button>
              </div>
              <textarea class="question-card-text" rows={1} placeholder="Note (optional)" aria-label="Note" value={form.note} onInput={e => setForm('note', e.currentTarget.value)} />
            </fieldset>
          )}
        </Match>

        <Match when={request().mode === 'checkbox' && request().checkbox}>
          {(c) => (
            <fieldset class="question-card-q" disabled={stale()}>
              <legend class="question-card-prompt">{c().prompt}</legend>
              <Show when={c().detailsMarkdown}><MarkdownView text={c().detailsMarkdown!} class="question-card-md" /></Show>
              <Show when={c().target}><p class="question-card-target"><code>{c().target!.path}</code></p></Show>
              <div class="question-card-options">
                <For each={c().options ?? []}>
                  {(option) => (
                    <label class="question-card-option" for={idFor('cb', option.id)}>
                      <input
                        id={idFor('cb', option.id)}
                        type="checkbox"
                        checked={form.selected.includes(option.id)}
                        onChange={() => setForm('selected', list => list.includes(option.id) ? list.filter(id => id !== option.id) : [...list, option.id])}
                      />
                      <span class="question-card-option-label">{option.label}</span>
                      <Show when={option.description}><span class="question-card-option-desc">{option.description}</span></Show>
                    </label>
                  )}
                </For>
              </div>
              <Show when={errors().checkbox}><p class="question-card-field-error">{errors().checkbox}</p></Show>
            </fieldset>
          )}
        </Match>

        <Match when={request().mode === 'verdicts' && request().verdicts}>
          {(v) => (
            <fieldset class="question-card-q" disabled={stale()}>
              <legend class="question-card-prompt">{v().prompt}</legend>
              <Show when={v().detailsMarkdown}><MarkdownView text={v().detailsMarkdown!} class="question-card-md" /></Show>
              <For each={v().items ?? []}>
                {(item) => (
                  <div class="question-card-item">
                    <div class="question-card-item-label">{item.label}</div>
                    <Show when={item.description}><p class="question-card-help">{item.description}</p></Show>
                    <Show when={item.previewMarkdown}><MarkdownView text={item.previewMarkdown!} class="question-card-md" /></Show>
                    <div class="question-card-choices">
                      <For each={v().verdicts ?? []}>
                        {(verdict) => (
                          <button
                            type="button"
                            class="question-card-choice"
                            classList={{ on: form.verdicts[item.id]?.verdict === verdict }}
                            aria-pressed={form.verdicts[item.id]?.verdict === verdict}
                            onClick={() => setForm('verdicts', item.id, 'verdict', verdict)}
                          >{VERDICT_LABELS[verdict]}</button>
                        )}
                      </For>
                    </div>
                    <Show when={(v().requireReasonOn ?? []).includes(form.verdicts[item.id]?.verdict as InteractionVerdict)}>
                      <input
                        class="question-card-other"
                        type="text"
                        placeholder="Reason"
                        aria-label={`${item.label}: reason`}
                        value={form.verdicts[item.id]?.reason ?? ''}
                        onInput={e => setForm('verdicts', item.id, 'reason', e.currentTarget.value)}
                      />
                    </Show>
                    <Show when={errors()[item.id]}><p class="question-card-field-error">{errors()[item.id]}</p></Show>
                  </div>
                )}
              </For>
              <Show when={v().allowBulkApprove !== false && (v().verdicts ?? []).includes('approve')}>
                <button type="button" class="question-card-btn" onClick={() => setForm(state => approveAll(request(), state))}>Approve all</button>
              </Show>
            </fieldset>
          )}
        </Match>
      </Switch>

      <Show when={props.error}>
        <p class="question-card-error" role="alert">{props.error}</p>
      </Show>
      <div class="question-card-btns">
        <button type="button" class="question-card-btn dismiss" disabled={props.submitting} onClick={() => props.onDismiss()}>Dismiss</button>
        <button type="submit" class="question-card-btn submit" disabled={!canSubmit()}
          aria-describedby={props.blockReason ? idFor('block') : props.asMessage ? idFor('as-message') : undefined}>
          {request().submitLabel || 'Submit answers'}
        </button>
      </div>
    </form>
  );
}
