import { For, Show, createEffect, createMemo, createSignal, on, type JSX } from 'solid-js';
import { createStore, produce } from 'solid-js/store';
import { Icon } from '../icons';
import { QuestionCard } from './QuestionCard';
import { AS_MESSAGE_NOTE, buildAnswer, cardTitle, initialFormState, questionLabel, questionReason, questionScenario, type QuestionFormState } from '../questionCards';
import { answerWords, dockKey, dockRows, questionCount, startCursor, stepError, tooSoon, waitingWords, type DockAction } from '../questionDock';
import type { ChatInteraction, InteractionAnswer, InteractionQuestion } from '../types';
import { ErrorText } from './ui';
import '../styles/question-card.css';

/**
 * Docked question cards (2026-10-14): the agent's questions sit where the
 * message box is, one at a time, as Claude Code asks them. Choosing an
 * answer moves to the next; the last one sends the answers straight to the
 * agent as the next message ("Answered questions", then each question
 * and its answer). Back revisits; "Other…" takes the person's own words; the
 * recommended option is marked and the keyboard starts on it. Esc hides the
 * dock and brings the message box back; a chip over it ("2 questions
 * waiting") opens the dock again. Keys are plain (`questionDock.ts`) and
 * every one has a button.
 *
 * Only `questions` cards step; a confirmation, checkbox or verdicts card is
 * one step, drawn whole by `QuestionCard` inside the dock.
 */
export interface QuestionDockProps {
  card: ChatInteraction;
  /** Why Send can't go now. Shown from the first question, before any
   *  answer is picked (2026-11-04). */
  blockReason: string | null;
  /** The answers go as an ordinary message: the conversation no longer
   *  resumes the session that asked. Said on the first question. */
  asMessage?: boolean;
  submitting: boolean;
  error: string | null;
  onSubmit: (answer: InteractionAnswer) => void;
  onDismiss: () => void;
  onHide: () => void;
  /** Hidden behind the "questions waiting" chip: kept mounted, so what the
   *  person already answered stays. */
  hidden?: boolean;
}

export function QuestionDock(props: QuestionDockProps) {
  let root: HTMLDivElement | undefined;
  // Esc anywhere in the dock hides it, unless a field inside took it first.
  const onKeyDown: JSX.EventHandler<HTMLDivElement, KeyboardEvent> = (e) => {
    if (e.key !== 'Escape' || e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    e.preventDefault();
    props.onHide();
  };
  // Shown (a new card, or the chip), it takes focus unless the person is
  // typing in a field elsewhere (the hidden message box doesn't count).
  createEffect(on(() => !!props.hidden, (hidden) => {
    if (hidden) return;
    const active = document.activeElement as HTMLElement | null;
    const typing = !!active && !root?.contains(active) && !active.closest('.chat-input')
      && (active.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(active.tagName));
    if (!typing) queueMicrotask(() => root?.querySelector<HTMLElement>('[data-dock-focus]')?.focus());
  }));
  return (
    <div class="question-dock" ref={root} hidden={props.hidden} role="region" aria-label={`The agent is asking: ${cardTitle(props.card)}`} data-interaction-id={props.card.id} onKeyDown={onKeyDown}>
      <Show when={props.card.request.mode === 'questions'} fallback={
        <div class="question-dock-whole">
          <QuestionCard card={props.card} blockReason={props.blockReason} asMessage={props.asMessage} submitting={props.submitting} error={props.error}
            onSubmit={props.onSubmit} onDismiss={props.onDismiss} />
          <div class="question-dock-foot">
            <button type="button" class="btn ghost sm" data-dock-focus onClick={() => props.onHide()} title="Hide the questions and show the message box (Esc)">Hide</button>
          </div>
        </div>
      }>
        <QuestionSteps {...props} />
      </Show>
    </div>
  );
}

function QuestionSteps(props: QuestionDockProps) {
  const request = () => props.card.request;
  const questions = () => request().questions ?? [];
  const [form, setForm] = createStore<QuestionFormState>(initialFormState(props.card.request));
  const [step, setStep] = createSignal(0);
  const [cursor, setCursor] = createSignal(0);
  const [stepNote, setStepNote] = createSignal<string | null>(null);
  const stale = () => props.card.status === 'stale';
  const q = () => questions()[step()] as InteractionQuestion | undefined;
  const rows = createMemo(() => { const x = q(); return x ? dockRows(x) : []; });
  const last = () => step() >= questions().length - 1;
  const answer = (id: string) => form.answers[id];
  const id = (...parts: string[]) => `qd-${props.card.id}-${parts.join('-')}`;
  // `:` cannot occur in a question id, so these never match an id() id.
  const scenarioId = (qid: string) => `qd-${props.card.id}-scenario:${qid}`;
  const outcomeId = (qid: string, row: number) => `qd-${props.card.id}-outcome:${qid}:${row}`;
  let list: HTMLDivElement | undefined;
  let shownAt = Date.now();
  createEffect(on(() => !!props.hidden, (hidden) => { if (!hidden) shownAt = Date.now(); }));
  let other: HTMLInputElement | undefined;
  let text: HTMLTextAreaElement | undefined;

  // Each step starts on its recommended option (or what was chosen before,
  // coming Back), and focus follows the step.
  createEffect(on(step, () => {
    const x = q();
    if (!x) return;
    const a = answer(x.id);
    const chosen = a?.selected.length ? rows().findIndex(r => r.kind === 'option' && a.selected.includes(r.id)) : a?.otherOn ? rows().findIndex(r => r.kind === 'other') : -1;
    setCursor(chosen >= 0 ? chosen : startCursor(x));
    setStepNote(null);
  }));
  const focusStep = () => queueMicrotask(() => (x => x?.focus())(q()?.answerMode === 'text' ? text : answer(q()?.id ?? '')?.otherOn && q()?.answerMode === 'single' ? other : list));

  function go(to: number) {
    if (to < 0 || to >= questions().length) return;
    setStep(to);
    focusStep();
  }

  /** Go on from this step: the next question, or send them all. */
  function next() {
    const x = q();
    if (!x || stale()) return;
    const problem = stepError(x, answer(x.id));
    if (problem) { setStepNote(problem); return; }
    setStepNote(null);
    if (!last()) return go(step() + 1);
    if (props.blockReason || props.submitting) return;
    props.onSubmit(buildAnswer(request(), form));
  }

  function choose(index: number) {
    const x = q();
    const row = rows()[index];
    if (!x || !row || stale()) return;
    setCursor(index);
    if (row.kind === 'other') {
      setForm(produce(s => {
        const a = s.answers[x.id];
        if (!a) return;
        a.otherOn = x.answerMode === 'single' ? true : !a.otherOn;
        if (x.answerMode === 'single') a.selected = [];
      }));
      if (answer(x.id)?.otherOn) queueMicrotask(() => other?.focus());
      return;
    }
    setForm(produce(s => {
      const a = s.answers[x.id];
      if (!a) return;
      if (x.answerMode === 'single') { a.selected = [row.id]; a.otherOn = false; }
      else a.selected = a.selected.includes(row.id) ? a.selected.filter(v => v !== row.id) : [...a.selected, row.id];
    }));
    if (x.answerMode === 'single') next();
  }

  function act(a: DockAction) {
    switch (a.type) {
      case 'move': setCursor(a.to); break;
      case 'pick': choose(a.index); break;
      case 'confirm': next(); break;
      case 'back': go(step() - 1); break;
      case 'hide': props.onHide(); break;
    }
  }

  const onListKey = (e: KeyboardEvent) => {
    const x = q();
    if (!x || x.answerMode === 'text') return;
    const a = dockKey(e, { mode: x.answerMode === 'multi' ? 'multi' : 'single', rows: rows().length, cursor: cursor() });
    if (!a) return;
    // Keys meant for the message box a moment ago never answer (review):
    // in the first half second only moving and hiding work (`tooSoon`).
    if ((a.type === 'pick' || a.type === 'confirm') && tooSoon(shownAt, Date.now())) { e.preventDefault(); return; }
    e.preventDefault();
    e.stopPropagation();
    act(a);
  };
  const onFieldKey = (e: KeyboardEvent & { currentTarget: HTMLElement }) => {
    if (e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return;
    // Enter in a text or "Other…" field waits out the same half second
    // (2026-10-15): before, it went on (or sent) at once.
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); if (!tooSoon(shownAt, Date.now())) next(); }
    // Esc in the "Other…" box goes back to the list; the list's Esc hides.
    else if (e.key === 'Escape' && e.currentTarget === other) { e.preventDefault(); e.stopPropagation(); list?.focus(); }
  };

  const hint = () => {
    const x = q();
    if (!x) return '';
    if (x.answerMode === 'text') return 'Enter to go on · Shift+Enter for a new line · Esc to hide';
    if (x.answerMode === 'multi') return '↑↓ to move · Space or 1–9 to tick · Enter to go on · Esc to hide';
    return '↑↓ to move · Enter or 1–9 to choose · Esc to hide';
  };
  const goLabel = () => (last() ? (request().submitLabel?.trim() || 'Send answers') : 'Next');

  return (
    <div class="question-dock-steps" classList={{ 'is-stale': stale() }}>
      <div class="question-dock-head">
        <Icon name="message" size={14} />
        <span class="question-dock-kicker">{stale() ? 'Out of date' : 'The agent is asking'}</span>
        <span class="question-dock-title">{cardTitle(props.card)}</span>
        <Show when={questions().length > 1}>
          <span class="question-dock-count" aria-live="polite">{step() + 1} of {questions().length}</span>
        </Show>
        <button type="button" class="icon-btn question-dock-hide" onClick={() => props.onHide()} title="Hide the questions and show the message box (Esc)" aria-label="Hide the questions">
          <Icon name="chevronDown" size={14} />
        </button>
      </div>
      <Show when={step() === 0 && request().description}>
        <p class="question-dock-desc">{request().description}</p>
      </Show>
      {/* What keeps Send from going, on every question from the first, so
          nobody fills in a card that can't go; and, on the first, when the
          answers go as a message instead of into the asking session. */}
      <Show when={props.blockReason}>
        <p class="question-card-block" id={id('block')}>{props.blockReason}</p>
      </Show>
      <Show when={step() === 0 && props.asMessage && !props.blockReason}>
        <p class="question-dock-desc question-card-as-message" id={id('as-message')} role="note">{AS_MESSAGE_NOTE}</p>
      </Show>
      {/* What was answered so far: each opens its step again. */}
      <Show when={step() > 0}>
        <ol class="question-dock-trail" aria-label="Answered so far">
          <For each={questions().slice(0, step())}>{(done, i) => (
            <li><button type="button" class="question-dock-trail-btn" onClick={() => go(i())} title="Change this answer">
              <span class="question-dock-trail-q">{questionLabel(done)}</span>
              <span class="question-dock-trail-a">{answerWords(done, answer(done.id))}</span>
            </button></li>
          )}</For>
        </ol>
      </Show>
      <Show when={q()} keyed>{(x) => (
        <div class="question-dock-q">
          <p class="question-dock-prompt" id={id(x.id, 'prompt')}>
            {x.prompt}
            <Show when={x.required === false}><span class="question-card-optional"> (optional)</span></Show>
          </p>
          <Show when={questionReason(x)}><p class="question-dock-help">{questionReason(x)}</p></Show>
          <Show when={questionScenario(x)}><p class="question-card-scenario" id={scenarioId(x.id)}>{questionScenario(x)}</p></Show>
          <Show when={x.answerMode !== 'text'} fallback={
            <textarea ref={text} class="question-card-text" rows={2} data-dock-focus aria-labelledby={id(x.id, 'prompt')} aria-describedby={questionScenario(x) ? scenarioId(x.id) : undefined} disabled={stale()}
              value={answer(x.id)?.text ?? ''} onInput={e => setForm('answers', x.id, 'text', e.currentTarget.value)} onKeyDown={onFieldKey} />
          }>
            <div ref={list} class="question-dock-options" role="listbox" tabindex="0" data-dock-focus
              aria-labelledby={id(x.id, 'prompt')} aria-describedby={questionScenario(x) ? scenarioId(x.id) : undefined}
              aria-multiselectable={x.answerMode === 'multi' ? 'true' : undefined}
              aria-activedescendant={rows().length ? id(x.id, String(cursor())) : undefined} aria-disabled={stale() ? 'true' : undefined}
              onKeyDown={onListKey}>
              <For each={rows()}>{(row, i) => {
                const on = () => {
                  const a = answer(x.id);
                  return row.kind === 'other' ? !!a?.otherOn : !!a?.selected.includes(row.id);
                };
                return (
                  <div id={id(x.id, String(i()))} class="question-dock-option" role="option" aria-selected={on() ? 'true' : 'false'}
                    aria-describedby={row.kind === 'option' && row.outcome ? outcomeId(x.id, i()) : undefined}
                    classList={{ 'is-cursor': cursor() === i(), 'is-on': on() }}
                    onClick={() => { list?.focus(); choose(i()); }} onMouseMove={() => setCursor(i())}>
                    <span class="question-dock-key" aria-hidden="true">{i() < 9 ? i() + 1 : ''}</span>
                    <Show when={x.answerMode === 'multi'}><span class="question-dock-tick" aria-hidden="true">{on() ? '☑' : '☐'}</span></Show>
                    <span class="question-dock-label">{row.label}</span>
                    <Show when={row.kind === 'option' && row.recommended}><span class="question-card-badge">Recommended</span></Show>
                    <Show when={row.kind === 'option' && row.description}><span class="question-dock-option-desc">{row.kind === 'option' ? row.description : ''}</span></Show>
                    <Show when={row.kind === 'option' && row.outcome}><span class="question-card-outcome question-dock-option-desc" id={outcomeId(x.id, i())} aria-hidden="true">{row.kind === 'option' ? row.outcome : ''}</span></Show>
                  </div>
                );
              }}</For>
            </div>
            <Show when={answer(x.id)?.otherOn}>
              <input ref={other} class="question-card-other" type="text" placeholder="Type your answer, then press Enter"
                aria-label={`${questionLabel(x)}: your answer`} value={answer(x.id)?.other ?? ''} disabled={stale()}
                onInput={e => setForm('answers', x.id, 'other', e.currentTarget.value)} onKeyDown={onFieldKey} />
            </Show>
          </Show>
          <Show when={stepNote()}>{(note) => <p class="question-card-field-error" role="alert">{note()}</p>}</Show>
        </div>
      )}</Show>
      <Show when={props.error}>{(err) => <ErrorText text={err()} class="question-card-error" />}</Show>
      <div class="question-dock-foot">
        <button type="button" class="btn ghost sm" disabled={step() === 0} onClick={() => go(step() - 1)}>Back</button>
        <Show when={q()?.required === false && !last()}>
          <button type="button" class="btn ghost sm" onClick={() => go(step() + 1)}>Skip</button>
        </Show>
        <span class="question-dock-hint">{hint()}</span>
        <button type="button" class="btn ghost sm question-dock-dismiss" onClick={() => props.onDismiss()} disabled={props.submitting}
          title="Close these questions without answering">Don’t answer</button>
        <Show when={q()?.answerMode !== 'single' || last() || answer(q()?.id ?? '')?.otherOn}>
          <button type="button" class="btn primary sm" disabled={stale() || props.submitting || (last() && !!props.blockReason)} onClick={() => next()}
            aria-describedby={props.blockReason ? id('block') : step() === 0 && props.asMessage ? id('as-message') : undefined}>
            {props.submitting ? 'Sending…' : props.error && last() ? 'Try again' : goLabel()}
          </button>
        </Show>
      </div>
    </div>
  );
}

/** Over the message box while the dock is hidden: how many questions wait,
 *  and the way back to them. */
export function QuestionsWaiting(props: { cards: ChatInteraction[]; onOpen: () => void }) {
  return (
    <div class="question-dock-chip-row">
      <button type="button" class="question-dock-chip" onClick={() => props.onOpen()} title="Show the agent’s questions">
        <Icon name="message" size={12} />
        <span>{waitingWords(questionCount(props.cards))}</span>
      </button>
    </div>
  );
}
