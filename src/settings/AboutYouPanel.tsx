/**
 * Settings, Personalization: the profile builder's scan controls and your
 * profile notes (`profile_builder.rs`).
 *
 * Hollow reads what the user's agents already know (Claude Code and Codex
 * instructions, Claude's memories about the user, Hollow's approved
 * memories, recent requests) and proposes facts for the five `me/` notes,
 * plus a few questions for what nobody knows yet. Since Settings PR 4
 * (overnight 11-12) the questions and suggestions are You items in Library,
 * Learnings (`components/ProfileSuggestions.tsx`); this panel keeps "Look
 * again", Keep learning, the sources, the notes with Mark reviewed, and one
 * "Waiting for you" line that opens Learnings. The first visit reads on its
 * own; "Keep learning" re-reads in the background as context grows.
 *
 * State is the shared plain-signal view in `store/profileBuilder.ts`, never a
 * resource, so an update can't suspend the Settings page. Lazy chunk with its
 * own CSS.
 */
import { For, Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import {
  defaultAgentLabel, flashFooter, openVaultNote, operatorProfile, refetchOperatorProfile, trackTask,
} from '../store';
import { openLibraryLearnings } from '../store/libraryMode';
import { enableApprovalsResource, reloadReviewQueue, reviewQueueFailed, reviewQueueItems } from '../store';
import { actProfile, profileBusy, profileLoadError, profileScanning, profileView, profileWaitingCount, scanProfile, useProfileBuilder } from '../store/profileBuilder';
import { learningsWaiting } from '../learningBulk';
import type { OperatorFreshness, ProfileFileView } from '../types';
import { headline, lastLooked, sourceChip } from '../aboutYou';
import { Icon } from '../icons';
import { Spinner } from '../components/Spinner';
import { Switch } from '../components/Switch';
import { errMsg } from '../utils';
import '../styles/connectors-library.css';
import '../styles/about-you.css';
import { Skeleton, ErrorText, PlainErrorText } from '../components/ui';

const FRESHNESS_TEXT: Record<string, string> = {
  fresh: 'current',
  due: 'review soon',
  overdue: 'review overdue',
  neverReviewed: 'not reviewed',
};

/** "Waiting for you: 3 profile suggestions, 2 learnings", one line that opens
 *  Library, Learnings where both wait (Settings PR 4). The learnings count
 *  reads the review queue snapshot Library's tab count reads; a count that
 *  isn't known yet says so instead of "0". */
export function WaitingForYouLine() {
  enableApprovalsResource();
  onMount(() => void reloadReviewQueue());
  const learnings = () => learningsWaiting(reviewQueueItems.latest, reviewQueueFailed());
  const noun = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  return (
    <button type="button" class="waiting-link about-you-waiting" data-suggestion-context="learning" onClick={() => void openLibraryLearnings()}>
      <Icon name="checkSquare" size={13} />
      <span class="waiting-link-title">Waiting for you</span>
      <span class="waiting-link-count">
        {noun(profileWaitingCount(), 'profile suggestion', 'profile suggestions')}, {learnings() === undefined && reviewQueueFailed() ? 'learnings count unavailable' : noun(learnings() ?? 0, 'learning', 'learnings')}
      </span>
      <span class="waiting-link-go">Open in Library, Learnings</span>
    </button>
  );
}

export function AboutYouPanel() {
  const view = profileView;
  const [reviewing, setReviewing] = createSignal<string | null>(null);
  const isScanning = profileScanning;
  const freshness = (path: string): OperatorFreshness | undefined =>
    operatorProfile()?.freshness?.find(f => f.path === path);
  const act = actProfile;
  const scan = scanProfile;
  const busy = profileBusy;
  const loadError = profileLoadError;
  useProfileBuilder({ autoScan: true });

  async function markReviewed(path: string) {
    if (reviewing()) return;
    setReviewing(path);
    try {
      await trackTask('Review operator context', () => api.markOperatorProfileReviewed(path), {
        detail: path,
        onSuccessResult: value => `${value.title} is current`,
      });
      await refetchOperatorProfile();
      flashFooter(`${path} marked reviewed`, 2200);
    } catch (e) {
      flashFooter(`× review update failed: ${errMsg(e)}`, 3500);
    } finally {
      setReviewing(null);
    }
  }

  return (
    <div class="about-you">
      <Show when={loadError()}>
        <ErrorText class="about-you-error" text={loadError()} />
      </Show>
      <Show when={view()} fallback={<Show when={!loadError()}><Skeleton label="Loading your profile" rows={4} /></Show>}>
        {v => (
          <>
            <WaitingForYouLine />
            <div class="about-you-hero">
              <div class="about-you-hero-copy">
                <div class="about-you-headline">{headline(v())}</div>
                <p class="about-you-muted">
                  Hollow reads what your agents already know about you, suggests what to add, and asks about the gaps.
                  Nothing goes into your profile until you add it.
                </p>
                <Show when={v().sources.length}>
                  <div class="about-you-sources" aria-label="Read from">
                    <span class="about-you-sources-k">Read from</span>
                    <For each={v().sources}>{s => <span class="about-you-chip">{sourceChip(s)}</span>}</For>
                  </div>
                </Show>
                <div class="about-you-meta">
                  <Show when={!isScanning()} fallback={<span>Reading now with {defaultAgentLabel()}</span>}>
                    <Show when={lastLooked(v().scannedAt)} fallback={<span>Hasn’t looked yet</span>}>
                      {when => <span>Last looked {when()}</span>}
                    </Show>
                  </Show>
                  <span class="about-you-dot" aria-hidden="true">·</span>
                  <span>Keep learning</span>
                  <Switch
                    checked={v().autoLearn}
                    label="Keep learning"
                    onChange={next => void act('auto', () => api.profileBuilderSetAutoLearn(next))}
                  />
                  <span class="about-you-hint">
                    {v().autoLearn ? 'Looks again every half day when there’s something new' : 'Only when you ask'}
                  </span>
                </div>
              </div>
              <button class="btn primary about-you-look" disabled={isScanning()} onClick={() => void scan()}>
                <Show when={isScanning()} fallback={<><Icon name="book" /> {v().scannedAt ? 'Look again' : 'Learn about me'}</>}>
                  <Spinner /> Reading…
                </Show>
              </button>
            </div>

            <Show when={isScanning()}>
              <div class="about-you-scanning" role="status">
                <Spinner /> Reading what Claude, Codex and Hollow already know about you. This takes about a minute.
              </div>
            </Show>
            <Show when={!isScanning() && v().lastError}>
              <p class="about-you-error" role="alert">Last look failed: <PlainErrorText text={v().lastError ?? ''} /></p>
            </Show>

            <Show when={v().scannedAt && !isScanning() && !v().questions.length && !v().suggestions.length && !v().lastError}>
              <p class="about-you-muted about-you-caught-up">You’re caught up. Hollow will suggest more as it learns.</p>
            </Show>

            <section class="about-you-block" aria-label="Your profile">
              <div class="about-you-block-head">
                <h3>Your profile</h3>
                <span class="about-you-muted">Every agent gets this, in every workspace and automation.</span>
              </div>
              <div class="about-you-notes">
                <For each={v().profile}>
                  {(note: ProfileFileView) => {
                    const f = () => freshness(note.path);
                    return (
                      <div class="about-you-note" data-state={note.exists ? (f()?.status ?? 'neverReviewed') : 'missing'}>
                        <div class="about-you-note-head">
                          <span class="about-you-note-title">{note.title}</span>
                          <span class="settings-state-chip" data-state={!note.exists ? undefined : f()?.status === 'fresh' ? 'ok' : f()?.status === 'overdue' ? 'error' : 'warn'}>
                            {note.exists ? FRESHNESS_TEXT[f()?.status ?? 'neverReviewed'] : 'not created'}
                          </span>
                        </div>
                        <Show when={note.items.length} fallback={<p class="about-you-muted">Nothing here yet.</p>}>
                          <ul class="about-you-note-items">
                            <For each={note.items}>{item => <li>{item}</li>}</For>
                          </ul>
                        </Show>
                        <Show when={note.blanks}>
                          <div class="about-you-target">{note.blanks} {note.blanks === 1 ? 'blank' : 'blanks'} to fill</div>
                        </Show>
                        <div class="about-you-note-actions">
                          <button class="btn ghost" disabled={!note.exists} onClick={() => openVaultNote(note.path)}>Edit</button>
                          <button
                            class="btn ghost"
                            disabled={!note.exists || reviewing() === note.path}
                            onClick={() => void markReviewed(note.path)}
                          >{reviewing() === note.path ? 'saving…' : 'Mark reviewed'}</button>
                        </div>
                      </div>
                    );
                  }}
                </For>
              </div>
            </section>
          </>
        )}
      </Show>
    </div>
  );
}

export default AboutYouPanel;
