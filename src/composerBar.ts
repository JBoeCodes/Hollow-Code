// The composer's bottom row (overnight 11-12, C3): + at left with what you
// add to a message, then the access-mode chip; the model picker, mic and
// Send at right. The lists here are pure so their order and wording are
// tested without mounting the composer (composerBar.test.ts).
import type { ActionMenuItem } from './components/ActionMenu';
import type { PermModeOption } from './permModes';

/** A composer command as ChatBody lists it (its slash popup's row). */
export interface ComposerCommand { name: string; desc: string; source: 'local' | 'claude'; hasArgs?: boolean }

/** Says why a command was not put in the box: the draft stays. */
export const KEEP_DRAFT = 'Your message stays: a command goes in an empty message box. Send or clear it first.';

/** The + menu's rows, in order: Attach files…, Add to Sources… (a
 *  workspace chat), Claude Code's own lists (or where another agent keeps
 *  them), the local slash commands and All commands…. */
export function composerAddItems(props: {
  agent: string;
  commands: readonly ComposerCommand[];
  busy: boolean;
  draft: string;
  onAttach: () => void;
  onMentionFile?: () => void;
  onMentionSkill?: () => void;
  onToggleMarkdown?: () => void;
  markdown?: boolean;
  onAddSource?: () => void;
  onInsert: (c: ComposerCommand) => void;
  onList: (name: '/mcp' | '/plugin') => void;
  onShowAll: () => void;
  onRefuse: (why: string) => void;
}): ActionMenuItem[] {
  const roomFor = () => !props.draft.trim() || /^\/\w*$/.test(props.draft);
  const insert = (c: ComposerCommand) => roomFor() ? props.onInsert(c) : props.onRefuse(KEEP_DRAFT);
  const local = props.commands.filter(c => c.source === 'local' && !['/mcp', '/plugin'].includes(c.name));
  const claude = props.agent === 'claude';
  return [
    { label: 'Attach files…', action: () => props.onAttach() },
    ...(props.onMentionFile ? [{ label: 'Mention a file…', action: props.onMentionFile }] : []),
    ...(props.onMentionSkill ? [{ label: 'Use a skill…', action: props.onMentionSkill }] : []),
    ...(props.onToggleMarkdown ? [{ label: props.markdown ? 'Use plain text' : 'Use live Markdown', action: props.onToggleMarkdown }] : []),
    ...(props.onAddSource ? [{ label: 'Add to Sources…', action: () => props.onAddSource?.() }] : []),
    ...(claude ? [
      { label: 'Claude Code’s MCP servers (its own list)', action: () => props.onList('/mcp'), disabled: props.busy, separatorBefore: true },
      { label: 'Claude Code’s plugins (its own list)', action: () => props.onList('/plugin'), disabled: props.busy },
    ] : [
      // Where it's needed, the alternative (X13): another agent's own
      // setup lives in its own CLI. Enabled, so the keyboard reaches it.
      { label: 'MCP-server and plugin lists: Claude chats only (switch in the model menu)', separatorBefore: true, action: () => props.onRefuse('Claude Code’s MCP servers and plugins are listed in a Claude chat: switch this chat to Claude in the model menu. Other agents keep theirs in their own CLI.') },
    ]),
    ...local.map((c, i) => ({ label: `${c.name} · ${c.desc}`, action: () => insert(c), separatorBefore: i === 0 })),
    { label: claude ? 'All commands, with Claude Code’s skills…' : 'All commands…', action: () => roomFor() ? props.onShowAll() : props.onRefuse(KEEP_DRAFT), separatorBefore: true },
  ];
}

/** The access chip's name for a screen reader: "Access: <mode>", with
 *  "(unrestricted)" in a danger mode, so it never reads like the model
 *  menu's own "Permissions: …" row. */
export function accessChipName(short: string, danger: boolean): string {
  return `Access: ${short}${danger ? ' (unrestricted)' : ''}`;
}

/** The access chip's menu: every permission mode in the pickers' order
 *  (the current one marked and not pickable, an unavailable one disabled
 *  and saying why in its words, both for a current mode this agent can't
 *  run), then the current mode's note as a read-only row. Each mode's description is its row's hover text. Nothing
 *  changes while a turn runs. */
export function accessMenuItems<M extends string>(props: {
  options: readonly PermModeOption<M>[];
  current: M;
  danger: Record<M, boolean>;
  locked: boolean;
  note: string;
  /** A last read-only line (Hollow's own cards on top of the mode). */
  footer?: string;
  onPick: (mode: M) => void;
}): ActionMenuItem[] {
  return [
    ...props.options.map(o => ({
      // The current mode reads current even when this agent can't run it:
      // the chip and the next send still use it (lane S re-review of #1304).
      label: `${o.label}${o.value === props.current ? ' · current' : ''}${o.disabled ? `: ${o.detail}` : ''}`,
      title: o.detail,
      action: () => props.onPick(o.value),
      disabled: props.locked || !!o.disabled || o.value === props.current,
      danger: props.danger[o.value],
    })),
    { label: props.locked ? `Access can change once this turn finishes. ${props.note}` : props.note, action: () => {}, informational: true, separatorBefore: true },
    ...(props.footer ? [{ label: props.footer, action: () => {}, informational: true }] : []),
  ];
}
