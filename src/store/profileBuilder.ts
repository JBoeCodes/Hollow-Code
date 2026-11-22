// The profile builder's one view (`profile_builder.rs`), shared by its two
// homes since Settings PR 4 (overnight 11-12): Personalization keeps the
// scan controls, the profile notes and the "Waiting for you" line; Library,
// Learnings shows the questions and suggestions as You items in its review
// queue. Both read this one view, so an Add in either shows in both, and
// accepting writes the same `me/` note through the same `profile_builder_*`
// calls as before.
//
// Plain signals loaded when a reader mounts, never a resource, so an update
// can't suspend the page.
import { createSignal, onCleanup, onMount } from 'solid-js';
import { api } from '../api';
import { listen, type UnlistenFn } from '../nativeEvents';
import { shouldAutoScan } from '../aboutYou';
import type { ProfileBuilderView } from '../types';
import { errMsg } from '../utils';
import { flashFooter } from './snackbars';
import { trackTask } from './activityDock';
import { refetchOperatorProfile } from './runtime';

const [view, setView] = createSignal<ProfileBuilderView | null>(null);
const [loadError, setLoadError] = createSignal('');
const [scanning, setScanning] = createSignal(false);
const [busy, setBusy] = createSignal<string | null>(null);

export const profileView = view;
export const profileLoadError = loadError;
/** A scan this window started, or one the backend reports running. */
export const profileScanning = () => scanning() || !!view()?.scanning;
/** The key of the action in flight (one at a time), or null. */
export const profileBusy = busy;

/** Suggestions plus questions: what "Waiting for you" counts. */
export const profileWaitingCount = () => (view()?.suggestions.length ?? 0) + (view()?.questions.length ?? 0);

export async function loadProfileBuilder(): Promise<ProfileBuilderView | null> {
  try {
    const next = await api.profileBuilderView();
    setView(next);
    setLoadError('');
    return next;
  } catch (e) {
    setLoadError(errMsg(e));
    return null;
  }
}

export async function scanProfile() {
  if (profileScanning()) return;
  setScanning(true);
  try {
    const next = await trackTask('Learn about you', () => api.profileBuilderScan(), {
      onSuccessResult: v => `${v.suggestions.length} to add · ${v.questions.length} questions`,
    });
    setView(next);
  } catch (e) {
    await loadProfileBuilder();
    flashFooter(`× couldn't read what your agents know: ${errMsg(e)}`, 4000);
  } finally {
    setScanning(false);
  }
}

/** Run one profile builder call, keep its returned view and re-read the
 *  profile notes. One at a time. */
export async function actProfile(key: string, fn: () => Promise<ProfileBuilderView>, done?: string) {
  if (busy()) return;
  setBusy(key);
  try {
    setView(await fn());
    await refetchOperatorProfile();
    if (done) flashFooter(done, 2200);
  } catch (e) {
    flashFooter(`× ${errMsg(e)}`, 3500);
  } finally {
    setBusy(null);
  }
}

/** Add a suggestion to its `me/` note, with the person's edit when they made one. */
export const acceptProfileSuggestion = (id: string, text?: string) =>
  actProfile(id, () => api.profileBuilderAccept(id, text), 'Added to your profile');

let readers = 0;
let stop: UnlistenFn | undefined;
let disposed = false;

/** Load the view and follow its changes while a reader is mounted. Only the
 *  Personalization panel asks for `autoScan` (the first visit reads on its own). */
export function useProfileBuilder(opts: { autoScan?: boolean } = {}) {
  let mounted = false;
  onMount(async () => {
    mounted = true;
    if (readers++ === 0) {
      disposed = false;
      listen('profile-builder-changed', () => {
        void loadProfileBuilder();
        void refetchOperatorProfile();
      }).then(fn => { if (disposed) fn(); else stop = fn; }).catch(() => { /* browser mock */ });
    }
    const first = await loadProfileBuilder();
    if (opts.autoScan && first && shouldAutoScan(first)) void scanProfile();
  });
  onCleanup(() => {
    if (!mounted || --readers > 0) return;
    disposed = true;
    stop?.();
    stop = undefined;
  });
}
