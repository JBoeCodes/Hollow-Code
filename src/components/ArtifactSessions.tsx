import { For, Show, createMemo, createResource, createSignal } from 'solid-js';
import { api } from '../api';
import type { ArtifactSessionLink } from '../types';
import { chatBusy, openVaultHistory } from '../store';
import { continueArtifactSession } from '../store/artifactSessions';
import { Skeleton, ErrorText } from './ui';
import { errMsg } from '../utils';

/** Lazy, file-scoped activity. Project callers pass the project note itself;
 * membership is never inferred from a matching name or prompt text. */
export function ArtifactSessions(props: { path: string; project?: boolean; onNavigate?: () => void }) {
  const [expanded, setExpanded] = createSignal(false);
  const [pending, setPending] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [links, { refetch }] = createResource(
    () => expanded() ? props.path : false,
    path => api.listArtifactSessions(path),
  );
  const groups = createMemo(() => {
    const grouped = new Map<string, ArtifactSessionLink[]>();
    if (links.error) return [];
    for (const link of links() ?? []) {
      const key = link.session?.key ?? link.run.id;
      grouped.set(key, [...(grouped.get(key) ?? []), link]);
    }
    return [...grouped.values()];
  });

  async function resume(link: ArtifactSessionLink, fromSummary = false) {
    if (!link.session || pending() || chatBusy()) return;
    setPending(link.session.key);
    setError(null);
    try {
      await continueArtifactSession(link.session.key, props.path, fromSummary);
      props.onNavigate?.();
    } catch (err) {
      setError(String(err instanceof Error ? err.message : err));
    } finally { setPending(null); }
  }

  return (
    <details class="artifact-sessions" onToggle={e => setExpanded(e.currentTarget.open)}>
      <summary>Sessions <span>Work that changed {props.project ? 'this project note' : 'this file'}</span></summary>
      <Show when={expanded()}>
        <div class="artifact-sessions-body">
          <div class="artifact-sessions-intro settings-row-v">
            <p>Linked by recorded file changes. History retains up to 200 runs for 90 days; older or external sessions may be absent.</p>
            <button type="button" class="btn small ghost" disabled={links.loading} onClick={() => { setError(null); void refetch(); }}>Refresh</button>
          </div>
          <Show when={error() || links.error}>
            <ErrorText class="artifact-sessions-error" text={error() || `Could not load sessions: ${errMsg(links.error)}`} />
          </Show>
          <Show when={links.loading}><Skeleton label="Loading recorded sessions" rows={2} /></Show>
          <Show when={!links.loading && !links.error && groups().length === 0}>
            <p class="artifact-sessions-empty">No retained chat runs changed this file. This does not mean no agent has worked on it.</p>
          </Show>
          <For each={groups()}>{group => {
            const first = group[0];
            const session = first.session;
            const native = session?.resumable === 'native' && !!session.harnessSessionId && ['claude', 'codex'].includes(session.agent);
            return (
              <article class="artifact-session-card">
                <div class="artifact-session-head">
                  <strong>{session?.title || (session ? 'Untitled session' : 'Earlier chat run')}</strong>
                  <span>{session?.agent || 'Session identity unavailable'}</span>
                </div>
                <For each={group}>{link => (
                  <div class="artifact-session-run settings-row-v">
                    <div>
                      <time dateTime={link.run.startedAt}>{new Date(link.run.startedAt).toLocaleString()}</time>
                      <span>{link.run.status} · {link.attribution === 'confirmed' ? 'Agent edit confirmed' : link.attribution === 'shared' ? 'Changed by another chat' : 'Change observed during run'}</span>
                    </div>
                    <button type="button" class="btn small ghost" onClick={() => {
                      openVaultHistory(props.path, link.run.id);
                      props.onNavigate?.();
                    }}>Review changes</button>
                  </div>
                )}</For>
                <Show when={session}>
                  <div class="artifact-session-actions">
                    <Show when={native}>
                      <button type="button" class="btn small" disabled={chatBusy() || !!pending()} onClick={() => void resume(first)}>
                        {pending() === session?.key ? 'Opening…' : 'Resume session'}
                      </button>
                    </Show>
                    <button type="button" class="btn small ghost" disabled={chatBusy() || !!pending()} onClick={() => void resume(first, true)}>
                      Continue from summary
                    </button>
                  </div>
                </Show>
              </article>
            );
          }}</For>
          <p class="artifact-sessions-footnote settings-row-v">Resume prepares the composer for review. Continue from summary starts a new conversation on your next send. Neither action sends automatically.</p>
        </div>
      </Show>
    </details>
  );
}
