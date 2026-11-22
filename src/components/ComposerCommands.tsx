import { ActionMenu, type ActionMenuItem } from './ActionMenu';
import { Icon } from '../icons';
import { composerAddItems, type ComposerCommand } from '../composerBar';
import '../styles/composer-commands.css';

/**
 * The composer's visible Commands menu (plan completion L4, review V02,
 * matrix N2). Every local command the slash popup offers is reachable by a
 * click here, not only by typing "/": choosing one puts it in the message
 * box (nothing runs until you send), and Claude Code's own MCP-server and
 * plugin lists run at once, read-only. Those two are Claude Code's own
 * setup, not Hollow's connectors (Settings, Connectors). "All commands…"
 * opens the slash list with the agent's skills too.
 *
 * Since 2026-11-01 it is the composer's + menu, "Attach and commands": its
 * first item attaches files (the separate Attach button is gone), so the
 * composer bar keeps one button for what you add to a message. Paste and
 * drag-and-drop attach too. Since C5 (2026-11-04) any file attaches, not
 * only an image, and a workspace chat's menu also has Add to Sources…,
 * which adds a file to the workspace's Sources instead of the message.
 * Overnight 11-12 (C3): its rows are built by `composerBar.ts`
 * (composerAddItems), so their order is tested without the composer.
 */
export function ComposerCommands(props: {
  agent: string;
  /** Opens the file picker (the composer's hidden file input). */
  onAttach: () => void;
  onMentionFile?: () => void;
  onMentionSkill?: () => void;
  onToggleMarkdown?: () => void;
  markdown?: boolean;
  extraItems?: ActionMenuItem[];
  /** Add to Sources… (a workspace chat only): the Sources file picker. */
  onAddSource?: () => void;
  commands: readonly ComposerCommand[];
  busy: boolean;
  /** What the message box holds now: a command goes into it only when it is
   *  empty (or holds just a "/…"), so a draft is never replaced. */
  draft: string;
  onInsert: (c: ComposerCommand) => void;
  onList: (name: '/mcp' | '/plugin') => void;
  onShowAll: () => void;
  /** Says why nothing was put in the box. */
  onRefuse: (why: string) => void;
}) {
  const items = () => [...composerAddItems(props), ...(props.extraItems ?? [])];
  return <span class="composer-commands"><ActionMenu label="Attach and commands" align="start" icon={<Icon name="plus" size={16} />} items={items()} /></span>;
}
