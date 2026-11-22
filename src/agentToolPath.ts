// Settings, Advanced, Folders agents can run tools from (A12): the words for
// `agent_tool_path`, kept pure so vitest covers them without a DOM.
import type { AgentToolPath, ShellPathRead, ToolPathFrom } from './generated/ipc';

/** One folder as the disclosure lists it. */
export interface ToolPathRow {
  dir: string;
  from: ToolPathFrom;
  /** The source, plus "missing" when the folder isn't there or "didn't
   *  answer" when its check timed out, so a muted row never relies on
   *  colour alone. */
  label: string;
  missing: boolean;
  /** The check didn't answer in time (a stalled network folder). */
  unknown: boolean;
}

/** Each entry in PATH order, labelled with where it came from. */
export function toolPathRows(path: AgentToolPath): ToolPathRow[] {
  return path.entries.map(e => ({
    dir: e.dir,
    from: e.from,
    label: e.exists === null ? `${e.from} · didn't answer` : e.exists ? e.from : `${e.from} · missing`,
    missing: e.exists === false,
    unknown: e.exists === null,
  }));
}

/** The PATH agents get, as the Copy button copies it. */
export function joinedToolPath(path: AgentToolPath): string {
  return path.entries.map(e => e.dir).join(':');
}

/** A plain note when the login shell's PATH couldn't be read or is still
 *  being read; null once it was read. */
export function shellPathNote(shell: ShellPathRead): string | null {
  if (shell.state === 'pending') return 'Still reading your shell\'s PATH.';
  if (shell.state === 'failed') {
    const detail = shell.detail?.trim();
    return `Hollow couldn't read your shell's PATH${detail ? ` (${detail})` : ''}. Agents get Hollow's default folders.`;
  }
  return null;
}
