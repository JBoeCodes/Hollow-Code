import { Show, createSignal, onMount } from 'solid-js';
import { api } from '../api';
import type { RunnerGithubStatus } from '../types';
import { errMsg, openExternal } from '../utils';
import { Button, ErrorText, Segmented } from '../components/ui';
import { MoreFold } from './settingsLayout';
import '../styles/runs-on.css';

/** Where a fine-grained token is made (GitHub's own page). */
export const RUNNER_TOKEN_URL = 'https://github.com/settings/personal-access-tokens/new';

/** Settings, Devices, Remote runner GitHub (remote runners, 2026-11-03):
 *  where a runner turn's GitHub credential comes from, when its workspace
 *  turns GitHub on (Runs on → On the runner). A runner token (the default:
 *  fine-grained, saved in this Mac's Keychain) or this Mac's gh login
 *  (broader). The credential stays on this Mac and reaches the runner one
 *  git or gh call at a time; it is never stored there. */
export function RunnerGithubRow() {
  const [status, setStatus] = createSignal<RunnerGithubStatus | null>(null);
  const [token, setToken] = createSignal('');
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');
  onMount(() => { void api.runnerGithubStatus().then(setStatus, e => setError(errMsg(e))); });
  async function act(run: () => Promise<RunnerGithubStatus>) {
    if (busy()) return;
    setBusy(true);
    setError('');
    try {
      setStatus(await run());
      setToken('');
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }
  const source = () => status()?.source ?? 'runner-token';
  return (
    <div class="settings-row runner-github">
      <div class="settings-row-label">
        <span class="settings-row-k" id="runner-github-k">Runner GitHub access</span>
        <div class="settings-row-v">
          Lets runner turns of a workspace that turns GitHub on push and open pull requests.
          <MoreFold>
            <div>
              It stays on this Mac and reaches the runner one git or gh call at a time; nothing is stored there.
              <Show when={source() === 'runner-token'} fallback={<> This Mac's gh login can do whatever that login can do, in every repository it reaches. {status()?.ghLogin ? 'gh is signed in here.' : "gh isn't signed in here (gh auth login in Terminal)."}</>}>
                {' '}Make a fine-grained token for it on GitHub: only the repositories the runner should push to, Repository permissions Contents and Pull requests set to Read and write, nothing else.
              </Show>
            </div>
          </MoreFold>
        </div>
        <Show when={source() === 'runner-token'}>
          <div class="runner-github-token">
            <Show when={status()?.tokenSaved} fallback={
              <>
                <input type="password" class="input runner-github-input" aria-labelledby="runner-github-k" placeholder="github_pat_…" autocomplete="off" spellcheck={false}
                  value={token()} onInput={e => setToken(e.currentTarget.value)} disabled={busy()} />
                <Button variant="primary" disabled={busy() || !token().trim()} onClick={() => void act(() => api.runnerGithubSaveToken(token()))}>Save token</Button>
                <Button variant="ghost" onClick={() => openExternal(RUNNER_TOKEN_URL)}>Make a token on GitHub</Button>
              </>
            }>
              <span class="runner-github-saved">A runner token is saved in this Mac's Keychain.</span>
              <Button variant="ghost" disabled={busy()} onClick={() => void act(() => api.runnerGithubClearToken())}>Remove token</Button>
            </Show>
          </div>
        </Show>
        <Show when={error()}>{msg => <ErrorText as="p" class="runs-on-error" text={msg()} />}</Show>
      </div>
      <Segmented label="Runner GitHub access" level="secondary"
        items={[{ value: 'runner-token', label: 'Runner token' }, { value: 'gh-login', label: "This Mac's gh login" }] as const}
        value={source()} onChange={next => void act(() => api.runnerGithubSetSource(next))} />
    </div>
  );
}
