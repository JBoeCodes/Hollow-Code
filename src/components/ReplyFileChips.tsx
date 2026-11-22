import { For, Show, createMemo } from 'solid-js';
import { Icon } from '../icons';
import { chatArtifacts, fileClick, openContextMenu, openVaultHistory, vaultRoot, type ContextMenuItem } from '../store';
import { footerChipPath, isVideoPath } from '../chatFileLinks';
import type { ExecutionReceipt } from '../executionReceipt';
import { fileKind } from '../fileChip';
import { openWorkspaceResult } from '../store/workspaces';
import { chipFolders, chipTarget, notesUpdate, type FileChip } from '../turnFooter';

/** The files a reply's run changed, as chips (overnight 11-12, lane C2):
 *  right-aligned above its "Worked for" line when its steps are folded, else
 *  above its row of actions. Each chip is the file's name with #1280's type
 *  icon (`fileChip.ts`), its path on hover, and its line counts when the
 *  receipt has them; a click opens it in the side panel. */
export interface ReplyFileChipsProps {
  receipt?: ExecutionReceipt;
  /** Whether the Run record is open under the reply (`+N more` toggles it). */
  detailsOpen: boolean;
  onToggleDetails: () => void;
  workspace?: { id: string; kind?: string } | null;
  /** The Main agent's conversation: its own brief and plan edits are one
   *  quiet line in the footer, not chips. */
  mainAgent?: boolean;
}

function chipIcon(chip: FileChip): { icon: import('../icons').IconName; tone: string } {
  if (chip.changeType === 'delete') return { icon: 'trash', tone: 'plain' };
  if (isVideoPath(chip.path)) return { icon: 'play', tone: 'video' };
  return fileKind(chip.path);
}

function Counts(props: { additions: number | null; deletions: number | null }) {
  return (
    <Show when={props.additions != null || props.deletions != null}>
      <span class="turn-chip-add">+{props.additions ?? 0}</span>
      <span class="turn-chip-del">−{props.deletions ?? 0}</span>
    </Show>
  );
}

export function ReplyFileChips(props: ReplyFileChipsProps) {
  const chips = createMemo(() => notesUpdate(props.receipt, props.mainAgent ? props.workspace?.id : undefined).chips);
  // Two chips with one file name say which folder each is in (EH4), also
  // when the twin is folded into `+N more` (A10).
  const folders = createMemo(() => {
    const { shown, folded } = chips();
    return chipFolders([...shown, ...folded], props.workspace?.id).slice(0, shown.length);
  });
  const folderOf = (chip: FileChip) => folders()[chips().shown.indexOf(chip)] ?? null;

  // Every chip opens the side panel's Preview (2026-11-04), a listed Result
  // with its review bar. A deleted one has nothing to show: a repository
  // row opens the Run record, a vault row its change in Library history.
  async function openChip(chip: FileChip) {
    if (chip.changeType === 'delete') {
      if (chip.root) props.onToggleDetails();
      else openVaultHistory(chip.path, props.receipt?.historyRunId);
      return;
    }
    // The conversation the click was in, before anything awaits.
    const owner = fileClick();
    const written = chip.root ? undefined
      : chatArtifacts().find(a => a.path === chip.path || a.path.endsWith(`/${chip.path}`));
    const path = written?.path ?? footerChipPath(chip, chip.root ? null : await vaultRoot());
    // A vault file gone since the reply offers its history.
    const history = () => openVaultHistory(chip.path, props.receipt?.historyRunId);
    const { openNamedFile } = await import('../store/fileOpen');
    if (path) await openNamedFile(path, null, { literal: true, owner, ...(chip.root ? {} : { missing: { label: 'Show its history', run: history } }) });
    else history();
  }

  // A Result chip's Review result opens it in its own workspace (also on the
  // reply's More actions, so it is never right-click only).
  function chipMenu(e: MouseEvent, chip: FileChip) {
    if (chip.root) return;
    e.preventDefault();
    const items: ContextMenuItem[] = [];
    const target = chipTarget(chip, props.workspace);
    if (target.to === 'results') items.push({ label: 'Review result', action: () => void openWorkspaceResult(target.workspaceId, target.path) });
    items.push({ label: 'Show change in history', action: () => openVaultHistory(chip.path, props.receipt?.historyRunId) });
    openContextMenu(e, items);
  }

  return (
    <Show when={chips().total > 0}>
      {/* A list of plain buttons: each in its own listitem, since a button
          can't be one (EH4). */}
      <div class="turn-chips-row">
      <div class="turn-chips" role="list" aria-label="Changed files">
        <For each={chips().shown}>
          {(chip) => {
            const kind = chipIcon(chip);
            return (
              <span role="listitem" class="turn-chip-item">
                <button
                  type="button"
                  class={`turn-chip tone-${kind.tone}`}
                  classList={{ deleted: chip.changeType === 'delete' }}
                  title={`${chip.root ? `${chip.root}/${chip.path}` : chip.path}\n${chip.changeType === 'delete' ? 'Show the change' : 'Preview'}`}
                  onClick={() => { void openChip(chip); }}
                  onContextMenu={e => chipMenu(e, chip)}
                >
                  <Icon name={kind.icon} size={13} />
                  <span class="turn-chip-name">{chip.name}</span>
                  <Show when={folderOf(chip)}>{(folder) => <span class="turn-chip-dir"> ({folder()})</span>}</Show>
                  <Counts additions={chip.additions} deletions={chip.deletions} />
                </button>
              </span>
            );
          }}
        </For>
        <Show when={chips().more}>
          {(more) => (
            <span role="listitem" class="turn-chip-item">
              <button
                type="button"
                class="turn-chip more"
                classList={{ active: props.detailsOpen }}
                aria-expanded={props.detailsOpen}
                title="Show every changed file"
                onClick={() => props.onToggleDetails()}
              >
                <Icon name="fileText" size={13} />
                <span class="turn-chip-name">+{more().count} more</span>
                <Counts additions={more().additions} deletions={more().deletions} />
              </button>
            </span>
          )}
        </Show>
      </div>
      </div>
    </Show>
  );
}
