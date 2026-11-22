import { For, Show, createMemo, createSignal, onMount } from 'solid-js';
import { Icon } from '../icons';
import { DiffCounts, FileChip, ToolCall, describeTool } from './ToolCall';
import { baseName, groupFiles, ledgerCounts, ledgerDefaultOpen, ledgerGlyphs, ledgerSummary } from '../toolDescribe';
import type { ChatToolUse } from '../store';
import {
  RAW_ROW_PX,
  RAW_VIRTUAL_THRESHOLD,
  foldTools,
  groupLabel,
  groupStatus,
  windowRange,
  type LedgerItem,
} from '../toolFolding';
import '../styles/tool-ledger.css';

/** Rows rendered beyond the visible window on each side of the Raw list. */
const RAW_OVERSCAN = 20;
/** The Raw list's viewport height before it is measured. */
const RAW_VIEWPORT_FALLBACK_PX = 420;

/**
 * One turn's tool calls (Paperclip PR 21). Its header summarizes them
 * (`ledgerSummary`): what was done, one small icon per call, then the live
 * call or the count ("7 tool calls, 1 subagent"). By default runs of three or more
 * same-family calls fold into one group row that expands to its members; a
 * call carrying an approval badge never folds. The per-turn Raw toggle shows
 * every call unfolded, and past `RAW_VIRTUAL_THRESHOLD` calls it becomes a
 * fixed-height windowed list of one-line summaries, so a 2,000-call turn
 * renders a few dozen rows. Folded rows render by their stable key
 * (`foldTools`), so while a turn streams a row is never rebuilt: a group that
 * gains a member keeps its DOM, its entrance does not replay and an expanded
 * group stays expanded. Only a real change of shape (two singles becoming a
 * group) swaps a row.
 */
export function ToolLedger(props: { tools: ChatToolUse[]; onToggle: (toolIdx: number) => void }) {
  const [raw, setRaw] = createSignal(false);
  const running = () => props.tools.some(t => t.status === 'running');
  const failing = () => props.tools.some(t => t.status === 'error');
  // The header says what the turn did ("Ran 2 commands, read ToolLedger.tsx")
  // and, while a call runs, which one.
  const summary = createMemo(() => ledgerSummary(props.tools));
  // One small icon per call, like a receipt of what kinds of work ran.
  const glyphs = createMemo(() => ledgerGlyphs(props.tools));

  // Open follows `ledgerDefaultOpen` until the user toggles it; after that
  // their choice stands. A finished turn no longer slams its ledger shut.
  const [userOpen, setUserOpen] = createSignal<boolean | undefined>(undefined);
  const open = () => userOpen() ?? ledgerDefaultOpen(props.tools.length, running());

  const items = createMemo(() => foldTools(props.tools));
  const byKey = createMemo(() => new Map(items().map(item => [item.key, item] as const)));
  // Strings: <For> keeps one row per key for as long as the key exists.
  const keys = createMemo(() => items().map(item => item.key));

  // One call (the common case natively, where each agent message is its own
  // turn) is just its row: no header repeating it, no count, no Raw toggle.
  // A turn that gains a second call becomes the full ledger (frontier F1).
  const single = () => props.tools.length === 1;

  return (
    <Show when={!single()} fallback={
      <div class="toolcalls chat-tool-ledger single">
        <For each={keys()}>
          {(key) => <LedgerRow item={() => byKey().get(key)} onToggle={props.onToggle} />}
        </For>
      </div>
    }>
    <details
      class="toolcalls chat-tool-ledger"
      open={open()}
      onToggle={(e) => {
        // Fired for our own `open` writes too; only a change is the user's.
        const now = e.currentTarget.open;
        if (now !== open()) setUserOpen(now);
      }}
    >
      <summary class="ledger-cap toolcalls-cap tool-ledger-cap">
        <span class="tool-ledger-cap-icon"><Icon name={summary().icon} size={13} /></span>
        <span class="tool-ledger-cap-text" title={summary().text}>{summary().text}</span>
        <span class="tool-ledger-glyphs" aria-hidden="true">
          <For each={glyphs().glyphs}>
            {(g) => (
              <span class={`tool-ledger-glyph st-${g.status}`} title={g.title}><Icon name={g.icon} size={12} /></span>
            )}
          </For>
          <Show when={glyphs().more > 0}><span class="tool-ledger-glyph-more">+{glyphs().more}</span></Show>
        </span>
        <span class="tool-ledger-cap-spacer" />
        {/* The live call is in the rows when the ledger is open. */}
        <Show when={!open() && summary().running}>
          {(now) => (
            <span class="tool-ledger-cap-now" title={`${now().verb} ${now().target}`}>
              <span class="live-dot on" aria-hidden="true" />
              {now().verb}
              <Show when={now().target}> <span class="tool-ledger-cap-now-target">{now().target}</span></Show>
            </span>
          )}
        </Show>
        <span class="tool-ledger-cap-meta">
          {ledgerCounts(props.tools)}
          <Show when={failing()}> · <span class="tool-ledger-cap-failed">{summary().failed} failed</span></Show>
        </span>
      </summary>
      <div class="tool-ledger-bar">
        <button
          type="button"
          class="tool-ledger-raw-toggle"
          aria-pressed={raw()}
          title="Show every action unfolded"
          onClick={() => setRaw(v => !v)}
        >Raw</button>
      </div>
      <Show
        when={raw()}
        fallback={
          <For each={keys()}>
            {(key) => <LedgerRow item={() => byKey().get(key)} onToggle={props.onToggle} />}
          </For>
        }
      >
        <Show
          when={props.tools.length > RAW_VIRTUAL_THRESHOLD}
          fallback={
            <For each={props.tools}>
              {(tu, i) => <ToolCall tool={tu} expanded={tu.expanded} onToggle={() => props.onToggle(i())} />}
            </For>
          }
        >
          <RawList tools={props.tools} />
        </Show>
      </Show>
    </details>
    </Show>
  );
}

type GroupItem = Extract<LedgerItem<ChatToolUse>, { kind: 'group' }>;
type SingleItem = Extract<LedgerItem<ChatToolUse>, { kind: 'single' }>;

/** One folded-view row. Its key fixes its kind (`g:` group, `t:` single);
 *  the item it reads updates in place as the turn streams. The accessor may
 *  briefly return undefined while <For> retires a key. */
function LedgerRow(props: { item: () => LedgerItem<ChatToolUse> | undefined; onToggle: (toolIdx: number) => void }) {
  const first = props.item();
  if (first?.kind === 'group') {
    const group = () => {
      const item = props.item();
      return item?.kind === 'group' ? item : undefined;
    };
    return <ToolGroupRow group={group} first={first} onToggle={props.onToggle} />;
  }
  const single = () => {
    const item = props.item();
    return item?.kind === 'single' ? item : undefined;
  };
  return (
    <Show when={single()}>
      {(item: () => SingleItem) => (
        <ToolCall tool={item().tool} expanded={item().tool.expanded} onToggle={() => props.onToggle(item().index)} />
      )}
    </Show>
  );
}

/** A folded run: one header ("Read 4 files") that expands to its calls. It
 *  stays mounted as members join, so the open state lives here safely. */
function ToolGroupRow(props: {
  group: () => GroupItem | undefined;
  first: GroupItem;
  onToggle: (toolIdx: number) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const tools = () => props.group()?.tools ?? [];
  const start = () => props.group()?.start ?? props.first.start;
  const status = createMemo(() => groupStatus(tools()));

  // Resolved once: <Icon> reads its name at setup. The family (and so the
  // icon) is fixed by the first call, which keys the row.
  const icon = describeTool(props.first.tools[0]?.name ?? '', props.first.tools[0]?.input).icon;
  // The files the run touched, and what an edit run changed in total.
  const files = createMemo(() => groupFiles(tools()));
  // An edit run counts files, not calls: three edits to two files is
  // "Edited 2 files".
  const label = () => {
    const n = files().files.length + files().more;
    return props.first.family === 'edit' && n > 0
      ? `Edited ${n} ${n === 1 ? 'file' : 'files'}`
      : groupLabel(props.first.family, tools().length);
  };
  // The toggle's name: the label, the files beside it and the totals, as
  // the row reads (its own text is only the label).
  const toggleLabel = () => {
    const f = files();
    return [
      label(),
      ...f.files.map(x => baseName(x.path)),
      f.more > 0 ? `+${f.more}` : '',
      f.stats ? `+${f.stats.additions} −${f.stats.deletions}` : '',
      status() === 'done' ? '' : status(),
    ].filter(Boolean).join(' ');
  };

  return (
    <div class={`toolcall tool-group st-${status()}`} classList={{ open: open() }}>
      {/* As in ToolCall: the toggle stretches over the row, and the file
          chips sit beside it, never inside a button. */}
      <div class="toolcall-head">
        <button
          class="toolcall-toggle"
          type="button"
          aria-label={toggleLabel()}
          aria-expanded={open()}
          onClick={() => setOpen(v => !v)}
        >
          <span class="toolcall-icon"><Icon name={icon} size={13} /></span>
          <span class="toolcall-verb">{label()}</span>
        </button>
        <span class="tool-group-files">
          <For each={files().files}>{(f) => <FileChip path={f.path} />}</For>
          <Show when={files().more > 0}><span class="tool-group-more">+{files().more}</span></Show>
        </span>
        <Show when={files().stats}>
          {(st) => <DiffCounts additions={st().additions} deletions={st().deletions} />}
        </Show>
        <span class="toolcall-spacer" />
        <span class="toolcall-status" aria-label={status()}>
          <Show when={status() === 'running'}><span class="live-dot on" aria-hidden="true" /></Show>
          <Show when={status() === 'error'}><span class="toolcall-x">✕</span></Show>
        </span>
        <span class="toolcall-caret" aria-hidden="true"><Icon name={open() ? 'chevronDown' : 'chevronRight'} size={12} /></span>
      </div>
      <Show when={open()}>
        <div class="tool-group-members">
          <For each={tools()}>
            {(tu, n) => (
              <ToolCall tool={tu} expanded={tu.expanded} onToggle={() => props.onToggle(start() + n())} />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

/** Every call as a one-line summary in a fixed-height window: only the rows
 *  in view (plus overscan) exist in the DOM. */
function RawList(props: { tools: ChatToolUse[] }) {
  let el: HTMLDivElement | undefined;
  const [top, setTop] = createSignal(0);
  const [viewport, setViewport] = createSignal(RAW_VIEWPORT_FALLBACK_PX);
  onMount(() => {
    if (el && el.clientHeight > 0) setViewport(el.clientHeight);
  });
  const rows = createMemo(() => {
    const { start, end } = windowRange(top(), viewport(), RAW_ROW_PX, props.tools.length, RAW_OVERSCAN);
    const out: number[] = [];
    for (let i = start; i < end; i++) out.push(i);
    return out;
  });

  return (
    <div
      ref={el}
      class="tool-ledger-raw-list"
      role="list"
      aria-label={`All ${props.tools.length} actions`}
      onScroll={(e) => setTop(e.currentTarget.scrollTop)}
    >
      <div class="tool-ledger-raw-spacer" style={{ height: `${props.tools.length * RAW_ROW_PX}px` }}>
        <For each={rows()}>
          {(i) => <RawRow tool={props.tools[i]} index={i} />}
        </For>
      </div>
    </div>
  );
}

function RawRow(props: { tool: ChatToolUse | undefined; index: number }) {
  const meta = createMemo(() => describeTool(props.tool?.name ?? '', props.tool?.input));
  const status = () => props.tool?.status ?? 'done';
  return (
    <div
      class={`tool-ledger-raw-row st-${status()}`}
      role="listitem"
      style={{ top: `${props.index * RAW_ROW_PX}px`, height: `${RAW_ROW_PX}px` }}
    >
      <span class="tool-ledger-raw-n">{props.index + 1}</span>
      <span class="tool-ledger-raw-verb">{meta().verb}</span>
      <span class="tool-ledger-raw-target">{meta().target}</span>
      <span class="tool-ledger-raw-status">{status() === 'running' ? '…' : status() === 'error' ? '✕' : ''}</span>
    </div>
  );
}
