import { Show, createSignal, onMount, onCleanup } from 'solid-js';
import { api } from '../api';
import type { SelfInstallResult } from '../workspaceTypes';
import '../styles/self-install-notice.css';
import { onNative } from '../nativeEvents';
import { showChoice } from '../store/overlays';
import type { QuitRequest } from '../workspaceTypes';
import { errMsg } from '../utils';
import { flashWithUndo } from '../store/snackbars';

/** An install record (`last-result.json`) nobody has seen yet, until
 *  dismissed: from an older Hollow's install helper (Build PR B5). Since row
 *  U10 a Local build's install is announced by the Update button's notice
 *  ("Updated to the local build of …") and its record is written already
 *  acknowledged, so this shows only a record nothing else announced. This
 *  component also asks "Quit and stop running work?" for a quit request.
 *  A successful install whose running-app check failed carries the reason
 *  in `message` (review B·F02).
 *  A failure says the previous app is still installed only when the
 *  installer confirmed it (`state: 'failed'`, slice 6). */
export function selfInstallNoticeText(r: SelfInstallResult): string {
  const why = r.message || 'see the log';
  if (r.ok || r.state === 'installed') return `Installed the build from ${r.branch} (${r.commit}).${r.message ? ` ${r.message}` : ''}`;
  if (r.state === 'failed') return `Install failed: ${why}. The previous app is still installed.`;
  return `Install failed: ${why}. Hollow could not confirm which app is installed now; check /Applications/Hollow.app.`;
}

export function SelfInstallNotice() {
  let notices!: HTMLDivElement;
  const [error, setError] = createSignal('');
  let disposed = false;
  let asking = '', quitRevision = 0;
  const askQuit = async (request: QuitRequest) => {
    if (disposed || asking === request.id) return;
    asking = request.id;
    const names = request.names;
    try {
      const choice = await showChoice({ title: 'Quit and stop running work?', body: request.uncertain ? 'Running work could not be checked. Quitting may stop chats and automations. Quit anyway?' : `Quitting stops ${names.length} run${names.length === 1 ? '' : 's'}:\n${names.join('\n')}`, danger: true,
        confirmLabel: request.uncertain ? 'Quit anyway' : `Quit and stop ${names.length} run${names.length === 1 ? '' : 's'}`, alternateLabel: 'Keep running in the menu bar',
      });
      if (disposed || asking !== request.id) return;
      const next = await api.resolveQuit(request.id, choice === 'confirm' ? 'quit' : choice === 'alternate' ? 'tray' : 'cancel');
      if (asking === request.id) asking = '';
      if (next) void askQuit(next);
    } catch (e) { if (asking === request.id) { asking = ''; setError(errMsg(e)); } }
  };
  const readQuit = () => {
    const read = ++quitRevision;
    void api.pendingQuitRequest().then(p => { if (!disposed && read === quitRevision && p) void askQuit(p); }).catch(e => setError(errMsg(e)));
  };
  onMount(() => {
    // Keep sticky stack receipts above the quit-error banner.
    const measure = () => document.documentElement.style.setProperty('--install-banner-height', `${notices.getBoundingClientRect().height}px`);
    const observer = new ResizeObserver(measure);
    observer.observe(notices); measure();
    onCleanup(() => { observer.disconnect(); document.documentElement.style.removeProperty('--install-banner-height'); });
    onCleanup(onNative<QuitRequest>('quit-requested', request => { quitRevision++; void askQuit(request); }));
    readQuit();
    onCleanup(() => { disposed = true; });
  });
  onMount(() => {
    void api.selfInstallResult().then(result => {
      if (!result || disposed) return;
      const installed = (result.ok || result.state === 'installed') && !result.message;
      flashWithUndo({
        message: selfInstallNoticeText(result), detail: result.log ? `Log: ${result.log}` : undefined,
        kind: installed ? 'status' : 'error', ttlMs: 0,
        actions: [
          ...(result.log ? [{ label: 'Reveal log', run: async () => { await api.chatPreviewOpen(result.log, true); } }] : []),
        ],
        onDismiss: () => { void api.ackSelfInstallResult().catch(() => {}); },
      });
    }).catch(() => {});
  });
  return (
    <div class="self-install-notices" ref={notices}>
    <Show when={error()}>{e => <div class="self-install-notice failed" role="alert">{e()}<button class="btn small ghost" onClick={() => setError('')}>Dismiss</button></div>}</Show>
    </div>
  );
}
