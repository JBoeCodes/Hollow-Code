import { For, Show } from 'solid-js';
import { Icon } from '../icons';
import { MoreButton } from './MoreButton';
import { RunTimer } from './RunTimer';
import { RowMarks } from './RowMarks';
import { Renamable, renameChatFromRow } from './RowRename';
import { SessionIndicator } from './SessionIndicator';
import { openContextMenu, type ContextMenuItem } from '../store';
import { runningSince, sessionState } from '../store/sessionStates';
import { setRenamingKey } from '../store/rowRename';
import { childFoldLabel, flatDescendants, groupState } from '../railChildren';
import type { AgentSessionSummary } from '../types';
import { railChatDragSource } from '../railChatDrag';
import { railRowSummary, withRailDetails } from '../railRowDetails';
import { railRowFileTarget } from '../railRowFiles';
import '../styles/row-children.css';

type Kids = ReadonlyMap<string, readonly AgentSessionSummary[]>;

/** The fold button on a row that has nested chats: the count, a chevron, and its words. */
export function ChildToggle(props: { count: number; open: boolean; onToggle: () => void }) {
  return <button type="button" class="work-row-fold" aria-expanded={props.open} aria-label={childFoldLabel(props.count, props.open)} title={childFoldLabel(props.count, props.open)}
    onClick={e => { e.preventDefault(); e.stopPropagation(); props.onToggle(); }}>
    <span class="work-row-fold-count" aria-hidden="true">{props.count}</span><Icon name={props.open ? 'chevronDown' : 'chevronRight'} size={12}/>
  </button>;
}

/** When `key`, or a chat nested under it, started working: the earliest start. */
export function groupRunningSince(key: string, kids: Kids, seen: Set<string> = new Set()): number | null {
  if (seen.has(key)) return null;
  seen.add(key);
  let best = sessionState(key) === 'running' ? runningSince(key) : null;
  for (const c of kids.get(key) ?? []) {
    const at = groupRunningSince(c.key, kids, seen);
    if (at !== null && (best === null || at < best)) best = at;
  }
  return best;
}

/** The nested chats of `parent`, indented, each opening its own chat. `menu` is that chat's
 *  menu (its Rename calls `rename`); `open` opens it. */
export function ChildRows(props: { parent: string; kids: Kids; depth?: number; visible?: (s: AgentSessionSummary) => boolean; here: (key: string) => boolean; locked: () => boolean;
  open: (s: AgentSessionSummary) => void; menu: (s: AgentSessionSummary, rename: () => void) => ContextMenuItem[] }) {
  const depth = () => props.depth ?? 1;
  // The deepest indent lists every deeper chat flat, so none is hidden.
  const list = () => depth() < 3 ? props.kids.get(props.parent) ?? [] : flatDescendants(props.parent, props.kids);
  return <For each={list()}>{c => {
    const name = () => c.title?.trim() || 'Subagent chat';
    const state = () => groupState(c.key, props.kids, sessionState);
    const rename = () => setRenamingKey(c.key);
    const items = () => withRailDetails(props.menu(c, rename).filter(item => item.label !== 'Details'), c);
    return <>
      <Show when={!props.visible || props.visible(c)}><div class="work-tree-head work-child-row" classList={{ current: props.here(c.key) }} style={{ '--child-depth': String(depth()) }} data-session-key={c.key} data-rail-motion-key={c.key}
        {...railChatDragSource(() => c.key, name, items)} {...railRowSummary(() => c)} {...railRowFileTarget(() => c.key)}
        onContextMenu={e => openContextMenu(e, items())}>
        <Renamable id={c.key} value={name()} label={`Rename ${name()}`} commit={n => renameChatFromRow(c.key, n)}>
          <button data-key-item type="button" class="work-nav-row work-workspace-row" aria-current={props.here(c.key) ? 'true' : undefined} title={name()}
            disabled={props.locked() && !props.here(c.key)} onClick={e => { if (!e.ctrlKey) props.open(c); }} onDblClick={rename}>
            <SessionIndicator state={state()} size={8} decorative/><span class="work-workspace-title">{name()}</span>
            <RunTimer since={groupRunningSince(c.key, props.kids)}/>
            <RowMarks session={c} here={props.here(c.key)}/>
          </button>
        </Renamable>
        <MoreButton class="work-row-actions" label={`Actions for chat ${name()}`} items={items}/>
      </div></Show>
      <Show when={depth() < 3}><ChildRows {...props} parent={c.key} depth={depth() + 1}/></Show>
    </>;
  }}</For>;
}
