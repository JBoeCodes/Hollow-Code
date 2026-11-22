import { Show, For, Suspense, createMemo, lazy } from 'solid-js';
import { Icon } from '../icons';
import { Highlight } from './Highlight';
import { fileClick, flashFooter, openFileInSidePane, type ChatToolUse } from '../store';
import { approvalBadgeLabel } from '../chatApproval';
import { baseName, describeRow, describeTool, fileKind } from '../toolDescribe';
import { editDiff } from '../toolDiff';
import { subagentCall } from '../subagentsModel';
import { SubagentLine } from './SubagentsInChat';
import { AnimatedHeight, MotionCue } from './MotionKit';

// M3a follow-up: a routed turn's chip, loaded only for a route row (the
// receipt is read there, `routeReceipt.ts`, so none of it rides this chunk).
const RouteChip = lazy(() => import('./RouteChip'));

// ToolLedger and ChatBody's approval card import it from here.
export { describeTool };
export type { ToolMeta } from '../toolDescribe';

/** A file a call acted on, as a chip: its type icon and name, the full path
 *  on hover. It opens in the Preview pane. The chip sits beside the row's
 *  expand toggle, never inside it (a control inside a button is a nested
 *  control: axe nested-interactive, 2026-11-04), above the toggle's stretched
 *  layer, so its click (and Enter/Space) opens the file, not the row. */
export function FileChip(props: { path: string }) {
  const kind = fileKind(props.path);
  // Any path opens since 2026-11-04: a relative one (Codex, other agents) is
  // read against the chat's folders, and a Mac mini path maps back here.
  const openable = () => props.path.trim().length > 0;
  const open = (e: Event) => {
    if (!openable()) return;
    e.preventDefault();
    e.stopPropagation();
    // A tool's path as written (literal), its resolver from its own chunk,
    // for the conversation the click was in.
    const owner = fileClick();
    void import('../store/fileOpen').then(m => m.openNamedFile(props.path, null, { literal: true, owner }));
  };
  return (
    <span
      class={`toolcall-file tone-${kind.tone}`}
      classList={{ openable: openable() }}
      title={openable() ? `${props.path}\nPreview` : props.path}
      role={openable() ? 'button' : undefined}
      tabIndex={openable() ? 0 : undefined}
      aria-label={openable() ? `Preview ${baseName(props.path)}` : undefined}
      onClick={open}
      onKeyDown={e => { if (e.key === 'Enter' || e.key === ' ') open(e); }}
    >
      <Icon name={kind.icon} size={12} />
      <span class="toolcall-file-name">{baseName(props.path)}</span>
    </span>
  );
}

/** An edit's `+a −d`. */
export function DiffCounts(props: { additions: number; deletions: number }) {
  return (
    <span class="toolcall-counts">
      <span class="toolcall-add">+{props.additions}</span>
      <span class="toolcall-del">−{props.deletions}</span>
    </span>
  );
}

/**
 * One agent action, rendered as a first-class block: an icon + human-readable
 * header ("Edit [store.ts] +3 −1", "Read 40 lines [runtime.ts]", a command's
 * reason beside the command as a pill), a live status (running → done/error),
 * and — when expanded — the call's substance: a red/green diff for edits, the
 * new file for writes, the command for Bash, and the tool's result/output.
 *
 * Collapsed by default so a long agent turn reads as a tidy action log; the
 * full I/O is one click away.
 */
export function ToolCall(props: { tool: ChatToolUse; expanded?: boolean; onToggle: () => void }) {
  // A subagent's start or message is one compact line that opens the
  // Subagents tab (COO rebuild L3). A call's name never changes.
  if (subagentCall(props.tool.name, props.tool.input)) return <SubagentLine name={props.tool.name} input={props.tool.input} result={props.tool.result} status={props.tool.status} />;
  async function copy(value: string) {
    try { await navigator.clipboard.writeText(value); flashFooter('Copied', 1000); }
    catch { flashFooter('Copy failed. Try again.', 1500); }
  }
  const meta = createMemo(() => describeRow(props.tool.name, props.tool.input, props.tool.result, props.tool.resultLines));
  // A dispatcher's route: once that Mac took it, a chip opens the chat there.
  const routed = () => props.tool.name === 'mcp__hollow_app__run_action' && obj(props.tool.input).id === 'sessions.route_to_runner';
  const status = () => props.tool.status ?? 'done';
  const diff = createMemo(() => editDiff(props.tool.name, props.tool.input));
  const writeContent = createMemo(() =>
    props.tool.name === 'Write' ? str(obj(props.tool.input).content) : '',
  );
  const bashCmd = createMemo(() =>
    props.tool.name === 'Bash' ? str(obj(props.tool.input).command) : '',
  );
  // Show raw input JSON for tools we don't render specially (MCP servers,
  // unknown tools) — for the rest the header + diff/command already say it.
  const showInput = createMemo(() => {
    const known = KNOWN.has(props.tool.name);
    if (known) return false;
    const i = props.tool.input;
    return i != null && (typeof i !== 'object' || Object.keys(i as object).length > 0);
  });
  const resultLang = () => (props.tool.name === 'Read' ? langFromPath(str(obj(props.tool.input).file_path)) : '');
  const hasBody = createMemo(() =>
    !!(diff() || writeContent() || bashCmd() || showInput() || props.tool.result),
  );
  // The toggle's name says what the whole row says: the chip and the rest
  // sit beside it, so its own text is only the verb, and every Grep row
  // would be "Search" (Codex review, 2026-11-04).
  const toggleLabel = () => {
    const m = meta();
    return [
      m.verb,
      m.file ? baseName(m.file) : m.target,
      m.stats ? `+${m.stats.additions} −${m.stats.deletions}` : '',
      m.extra ? `· ${m.extra}` : '',
      props.tool.approval ? approvalBadgeLabel(props.tool.approval) : '',
      status() === 'done' ? '' : status(),
    ].filter(Boolean).join(' ');
  };

  return (
    <div class="toolcall" classList={{ open: !!props.expanded, [`st-${status()}`]: true }}>
      {/* The row: its toggle holds the icon and verb and stretches over the
          whole row (`.toolcall-toggle::after`), so a click anywhere on it
          expands the call; the file chip is a sibling above that layer. */}
      <div class="toolcall-head" classList={{ 'no-body': !hasBody() }}>
        <button class="toolcall-toggle" type="button" onClick={() => { if (hasBody()) props.onToggle(); }} aria-label={toggleLabel()} aria-disabled={!hasBody()} aria-expanded={hasBody() ? !!props.expanded : undefined}>
          <span class="toolcall-icon"><Icon name={meta().icon} size={13} /></span>
          <span class="toolcall-verb"><MotionCue kind="tool" active={status() === 'running'}>{meta().verb}</MotionCue></span>
        </button>
        <Show
          when={meta().file}
          fallback={
            <Show when={meta().target}>
              <span class="toolcall-target" classList={{ mono: meta().mono, 'toolcall-pill': !!meta().mono }}>{meta().target}</span>
            </Show>
          }
        >
          {(path) => <FileChip path={path()} />}
        </Show>
        <Show when={routed() && props.tool.result}>
          {(result) => <Suspense fallback={null}><RouteChip name={props.tool.name} input={props.tool.input} result={result()} /></Suspense>}
        </Show>
        <Show when={meta().stats}>
          {(st) => <DiffCounts additions={st().additions} deletions={st().deletions} />}
        </Show>
        <Show when={meta().extra}>
          <span class="toolcall-extra">· {meta().extra}</span>
        </Show>
        <span class="toolcall-spacer" />
        <Show when={props.tool.approval}>
          {(badge) => (
            <span class={`toolcall-approval ${badge()}`} role="status">{approvalBadgeLabel(badge())}</span>
          )}
        </Show>
        <span class="toolcall-status" aria-label={status()}>
          <Show when={status() === 'running'}><span class="live-dot on" aria-hidden="true" /></Show>
          <Show when={status() === 'error'}><span class="toolcall-x">✕</span></Show>
        </span>
        <Show when={hasBody()}>
          <span class="toolcall-caret" aria-hidden="true"><Icon name={props.expanded ? 'chevronDown' : 'chevronRight'} size={12} /></span>
        </Show>
      </div>

      <Show when={props.tool.images && props.tool.images.length > 0}>
        <div class="toolcall-images">
          <For each={props.tool.images!}>
            {(src) => (
              <button type="button" class="toolcall-img-open" title="Open in the Preview pane" aria-label="Preview generated image" onClick={() => openFileInSidePane(src)}>
                <img class="toolcall-img" src={src} alt="generated image" loading="lazy" />
              </button>
            )}
          </For>
        </div>
      </Show>

      <AnimatedHeight><Show when={props.expanded && hasBody()}>
        <div class="toolcall-body">
          <Show when={bashCmd()}>
            <button type="button" class="btn small" aria-label="Copy Bash command" onClick={() => void copy(bashCmd())}>Copy command</button>
            <pre class="toolcall-code"><code><Highlight code={bashCmd()} lang="bash" /></code></pre>
          </Show>

          <Show when={diff()}>
            <div class="toolcall-diff">
              <For each={diff()!}>
                {(ln) => (
                  <Show when={ln.t !== 'gap'} fallback={<div class="diff-gap">⋯</div>}>
                    <div class={`diff-line diff-${ln.t}`}>
                      <span class="diff-gutter">{ln.t === 'add' ? '+' : ln.t === 'del' ? '-' : ' '}</span>
                      <span class="diff-text">{ln.v || ' '}</span>
                    </div>
                  </Show>
                )}
              </For>
            </div>
          </Show>

          <Show when={writeContent()}>
            <div class="toolcall-section-label">new file</div>
            <pre class="toolcall-code"><code><Highlight code={writeContent()} lang={langFromPath(str(obj(props.tool.input).file_path))} /></code></pre>
          </Show>

          <Show when={showInput()}>
            <div class="toolcall-section-label">input</div>
            <pre class="toolcall-code"><code><Highlight code={prettyJson(props.tool.input)} lang="json" /></code></pre>
          </Show>

          <Show when={props.tool.result}>
            <div class="toolcall-section-label">{props.tool.isError ? 'error' : 'output'}</div>
            <Show when={props.tool.resultChars}>{n => <div class="toolcall-section-label">Showing the start of {n()} characters</div>}</Show>
            <Show when={props.tool.name === 'Bash' && !props.tool.resultChars}><button type="button" class="btn small" aria-label="Copy Bash output" onClick={() => void copy(props.tool.result!)}>Copy output</button></Show>
            <pre class="toolcall-result" classList={{ error: !!props.tool.isError }}>
              <code><Highlight code={props.tool.result!} lang={resultLang()} /></code>
            </pre>
          </Show>
        </div>
      </Show></AnimatedHeight>
    </div>
  );
}

// ── helpers ──

/** Tools whose header and body already say what they did; everything else
 *  also shows its raw input. */
const KNOWN = new Set([
  'Read', 'Write', 'Edit', 'MultiEdit', 'NotebookEdit', 'Bash',
  'Grep', 'Glob', 'WebFetch', 'WebSearch', 'Task', 'TodoWrite',
]);

function obj(v: unknown): Record<string, unknown> {
  return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
}
function str(v: unknown): string {
  return typeof v === 'string' ? v : '';
}
function prettyJson(v: unknown): string {
  try { return JSON.stringify(v, null, 2); } catch { return String(v); }
}

function langFromPath(p: string): string {
  const m = /\.([A-Za-z0-9]+)$/.exec(p);
  const e = (m ? m[1] : '').toLowerCase();
  const map: Record<string, string> = {
    ts: 'ts', tsx: 'tsx', js: 'js', jsx: 'jsx', mjs: 'js', cjs: 'js',
    json: 'json', rs: 'rust', py: 'python', sh: 'bash', bash: 'bash',
    zsh: 'bash', css: 'css', scss: 'scss', less: 'less',
    html: 'html', xml: 'xml', svg: 'svg', vue: 'vue',
  };
  return map[e] ?? '';
}
