import { For, Show, createSignal } from 'solid-js';
import { api } from '../api';
import { joinedToolPath, shellPathNote, toolPathRows } from '../agentToolPath';
import type { AgentToolPath } from '../generated/ipc';
import { Button, ErrorText } from '../components/ui';
import { flashFooter } from '../store';
import { errMsg } from '../utils';
import { MotionCue } from '../components/MotionKit';

/**
 * Settings, Advanced, Folders agents can run tools from (A12): the PATH every
 * agent CLI gets (`agent_tool_path`, the same merge `agent::cli_path` joins),
 * one row per folder with where it came from, missing folders (and ones whose
 * check didn't answer within 2 s) muted and marked, and a Copy button for the
 * joined PATH. Says so when the login
 * shell's PATH is still being read or couldn't be. Read-only; it reads when
 * opened, so Settings' launch makes no extra call.
 */
export function AgentToolPathPanel() {
  const [path, setPath] = createSignal<AgentToolPath | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal('');

  // One read at a time: reopening while a slow read runs doesn't stack
  // another (overnight Spare 1; each read waits at most 2 s per folder set).
  const load = async () => {
    if (busy()) return;
    setBusy(true);
    setError('');
    try { setPath(await api.agentToolPath()); }
    catch (e) { setError(errMsg(e)); }
    finally { setBusy(false); }
  };

  const copy = async () => {
    const p = path();
    if (!p) return;
    try { await navigator.clipboard.writeText(joinedToolPath(p)); flashFooter('PATH copied'); }
    catch (e) { setError(`Could not copy: ${errMsg(e)}`); }
  };

  const status = () => {
    if (busy() && !path()) return 'Reading the folders…';
    const p = path();
    return p ? shellPathNote(p.shell) ?? '' : '';
  };

  // A settings row whose `.settings-row-k` carries the label, so a Settings
  // search hit for it scrolls here like any other control.
  return (
    <div class="settings-row">
      <details class="settings-access-kinds agent-tool-path" onToggle={e => { if (e.currentTarget.open) void load(); }}>
        <summary><span class="settings-row-k">Folders agents can run tools from</span></summary>
        <p class="disk-note">The PATH every agent gets, in order. When a tool works in Terminal but is “command not found” for an agent, its folder is missing here.</p>
        {/* Always mounted, so the shell note is announced when it appears. */}
        <p class="disk-note" role="status"><MotionCue kind="tool" active={busy()}>{status()}</MotionCue></p>
        <Show when={error()}>{msg => <ErrorText class="settings-row-warn" text={msg()} />}</Show>
        <Show when={path()}>{p => <>
          <ul class="disk-folders" role="list" aria-label="Folders agents can run tools from, in order">
            <For each={toolPathRows(p())}>{row =>
              <li data-from={row.from} data-missing={row.missing ? '' : undefined} data-unknown={row.unknown ? '' : undefined}>
                <span class="disk-folder-path" classList={{ 'disk-note': row.missing || row.unknown }} title={row.dir}>{row.dir}</span>{' '}
                <span class="disk-folder-size">{row.label}</span>
              </li>}
            </For>
          </ul>
          <div class="settings-actions">
            <Button size="sm" aria-label="Copy the PATH agents get" onClick={() => void copy()}>Copy</Button>
          </div>
        </>}</Show>
      </details>
    </div>
  );
}
