import { For, Show, createEffect, createSignal, on, onCleanup, onMount, type JSX } from 'solid-js';
import { Icon } from '../icons';
import { AgentMark } from './AgentMark';
import { openPreview } from '../store/preview';
import { activeChatRecall, chatRecallTrace, chatLiveRunId, appendToChatDraft, chatAgentSessionKey, chatCwd, chatTurns } from '../store/chatRuntime';
import { openWorkspaceFile, workspaceDetail } from '../store/workspaces';
import { enableWorkflowsResource, flashFooter, workflowsList } from '../store/runtime';
import { chooseAndAttachSource } from '../store/sourceAttach';
import { createSubagentsRead } from '../store/subagents';
import { chatUsesComputer } from '../store/computerChat';
import { ComputerCard } from './ComputerCard';
import { CardAutomations } from './CardAutomations';
import { ServicesPanel } from './ServicesPanel';
import { SidePanelCardHeader } from './SidePanelCardHeader';
import { ProjectScriptsControl } from './ProjectScriptsControl';
import { SidePanelCardGit } from './SidePanelCardGit';
import { isQuickChats, type ServiceView } from '../workspaceTypes';
import { automationsCount, chatAutomations, serversCount } from '../cardAutomationsModel';
import { api } from '../api';
import { errMsg } from '../utils';
import { CARD_SECTIONS_KEY, parseCardSections, type CardSection, cardFileRows, cardGlyphs, cardOutputs, cardSources, cardSubagentWords, recallFeedbackPrompt, recallTraceWords, viewAllSourcesPath } from '../sidePanelModel';
import '../styles/side-panel.css';

/** Persistent chat summary. Opening a pane replaces it; closing restores it
 * without taking focus. Subagents uses the shared provider-neutral live read. */
export function SidePanelCard() {
  const [open, setOpen] = createSignal<CardSection[]>((() => {
    try { return parseCardSections(localStorage.getItem(CARD_SECTIONS_KEY)); } catch { return []; }
  })());
  const toggle = (id: CardSection) => {
    const next = open().includes(id) ? open().filter(x => x !== id) : [...open(), id];
    setOpen(next);
    try { localStorage.setItem(CARD_SECTIONS_KEY, JSON.stringify(next)); } catch { /* Keep the current view usable without storage. */ }
  };
  const Section = (p: { id: CardSection; title: string; count: string | number; children: JSX.Element }) => (
    <section class="side-card-section" aria-labelledby={`side-card-${p.id}`}>
      <h2 class="side-card-head">
        <button type="button" id={`side-card-${p.id}`} class="side-card-toggle"
          aria-expanded={open().includes(p.id)} aria-controls={open().includes(p.id) ? `side-card-${p.id}-body` : undefined} onClick={() => toggle(p.id)}>
          <Icon name={open().includes(p.id) ? 'chevronDown' : 'chevronRight'} size={14} />
          <span>{p.title}</span><span class="side-card-count">{p.count}</span>
        </button>
      </h2>
      <Show when={open().includes(p.id)}>
        <div id={`side-card-${p.id}-body`} class="side-card-body">{p.children}</div>
      </Show>
    </section>
  );
  const detail = () => { const d = workspaceDetail(); return d?.workspace.cwd === chatCwd() ? d : null; };
  const project = () => { const w = detail()?.workspace; return w && !isQuickChats(w) ? w : null; };
  const parent = () => chatAgentSessionKey();
  const read = createSubagentsRead(parent);
  const subagentWords = () => !parent() ? 'None yet' : read.view() ? cardSubagentWords(read.view()) : read.error() ? 'Could not refresh' : 'Reading…';
  // Recall of the turn running now (cleared when it ends), else of the
  // newest reply's receipt: the list stays after the turn, for the feedback.
  const recall = () => {
    const live = activeChatRecall();
    if (live.length) return live;
    // A turn in flight whose recall is empty (silence, or no match) shows an
    // empty list, never the previous reply's.
    if (chatLiveRunId()) return [];
    for (let i = chatTurns.length - 1; i >= 0; i--) if (chatTurns[i].role === 'assistant') return chatTurns[i].receipt?.recall ?? [];
    return [];
  };
  // The trace belongs to the newest turn, so it shows only beside that
  // turn's own list, never beside an older receipt's.
  const recallTrace = () => { const t = chatRecallTrace(); return t?.sessionKey === chatAgentSessionKey() ? t : undefined; };
  const outputs = () => cardOutputs(detail()?.files);
  const sources = () => cardSources(detail()?.files);
  const [attaching, setAttaching] = createSignal(false);
  // The two counts are read here, so a collapsed section still says what is running.
  const [services, setServices] = createSignal<ServiceView[]>([]);
  const readServices = () => { const cwd = chatCwd(); if (!cwd) { setServices([]); return; } api.listServices(cwd).then(setServices, () => {}); };
  createEffect(on(chatCwd, readServices));
  onMount(() => { enableWorkflowsResource(); const timer = setInterval(readServices, 10_000); onCleanup(() => clearInterval(timer)); });
  const automations = () => chatAutomations(workflowsList(), chatAgentSessionKey()).length;
  const createOutput = () => {
    appendToChatDraft('Create an output in outputs/: ');
  };
  const addSource = async () => {
    const id = project()?.id;
    if (!id || attaching()) return;
    try { if (await chooseAndAttachSource(id, setAttaching)) flashFooter('Source attached'); }
    catch (e) { flashFooter(`× Couldn't attach it: ${errMsg(e)}`, 5000); }
  };
  let host: HTMLElement | undefined;
  // Same-name files in one list show the folder that tells them apart, as
  // reply chips do (`chipFolders`, `.file-chip-folder`).
  const FileRow = (p: { row: ReturnType<typeof cardFileRows>[number] }) => {
    const row = p.row;
    return (
      <li><button type="button" class={`side-card-file tone-${row.tone}`} data-path={row.path} title={row.path} aria-label={row.label}
        onClick={() => void openWorkspaceFile(row.path)}><Icon name={row.icon} size={15} /><span>{row.name}</span><Show when={row.folder}><span class="file-chip-folder">{row.folder}</span></Show></button></li>
    );
  };
  // A copy takes the rows' full paths, not their names: the rows a selection
  // covers, else the focused row (⌘C on it).
  const copyPaths = (e: ClipboardEvent) => {
    if (!host || !e.clipboardData) return;
    const sel = document.getSelection();
    const rows = [...host.querySelectorAll<HTMLElement>('.side-card-file[data-path]')];
    const picked = sel && !sel.isCollapsed ? rows.filter(r => sel.containsNode(r, true)) : rows.filter(r => r === document.activeElement);
    if (!picked.length) return;
    e.preventDefault();
    e.clipboardData.setData('text/plain', picked.map(r => r.dataset.path).join('\n'));
  };
  return (
    <aside id="side-panel-card" ref={host} tabindex="0" class="side-card-host" aria-label="Chat summary" onCopy={copyPaths}>
      <div class="side-card">
        <SidePanelCardHeader project={!!project()} workspaceId={project()?.id ?? null} />
        <Show when={project()} keyed>{w => <ProjectScriptsControl workspaceId={w.id} />}</Show>
        <SidePanelCardGit />
          <Section id="outputs" title="Outputs" count={(detail()?.files ?? []).filter(f => f.kind === 'output').length}>
              <Show when={project()}><button type="button" class="side-card-add" aria-label="Create an output" title="Create an output: asks this chat for one in outputs/" onClick={createOutput}><Icon name="plus" size={15} /></button></Show>
            <Show when={project()} fallback={<p class="side-card-empty">Outputs appear in a project.</p>}>
            <Show when={outputs().length} fallback={<button type="button" class="side-card-empty" onClick={createOutput}>Create an output</button>}>
              <ul class="side-card-files"><For each={cardFileRows(outputs().map(f => f.path))}>{row => <FileRow row={row} />}</For></ul>
            </Show>
            </Show>
          </Section>
        <Section id="subagents" title="Subagents" count={subagentWords()}>
          <button type="button" class="side-card-subagents" disabled={!parent()} title={parent() ? 'Open the Subagents tab' : 'Send a message first: subagents belong to a chat.'}
            aria-label={`Subagents: ${subagentWords()}`} onClick={() => { const p = parent(); if (p) openPreview({ kind: 'subagents', parent: p }); }}>
            <span class="side-card-marks" aria-hidden="true"><For each={cardGlyphs(read.view())}>{agent => <AgentMark agent={agent} size={16} />}</For></span>
            <span aria-live="polite" aria-atomic="true">{subagentWords()}<Show when={read.error() && read.view()}> (could not refresh)</Show></span>
          </button>
        </Section>
        <Show when={project()}>{w => (
          <Section id="sources" title="Sources" count={(detail()?.files ?? []).filter(f => f.kind === 'source').length}>
              <button type="button" class="side-card-add" aria-label="Add a source" title="Add a source: attach a local file as a snapshot" disabled={attaching()} onClick={() => void addSource()}><Icon name="plus" size={15} /></button>
            <ul class="side-card-files">
              <For each={cardFileRows(sources().map(f => f.path))}>{row => <FileRow row={row} />}</For>
              <li><button type="button" class="side-card-file side-card-all" title="All sources in Files" onClick={() => void openWorkspaceFile(viewAllSourcesPath(w().path, detail()?.files))}><Icon name="link" size={15} /><span>View all</span></button></li>
            </ul>
          </Section>
        )}</Show>
        {/* Parity 5.5: what this chat left running, and what runs in it on a schedule. */}
        <Show when={chatCwd()}>{cwd => (
          <Section id="servers" title="Servers" count={serversCount(services())}>
            <ServicesPanel cwd={cwd()} compact />
          </Section>
        )}</Show>
        <Section id="automations" title="Automations" count={automationsCount(automations())}>
          <CardAutomations />
        </Section>
        {/* Cua CU8: only once this chat has used the desktop tools. */}
        <Show when={chatUsesComputer()}>
          <Section id="computer" title="Computer" count={open().includes('computer') ? '' : 'Desktop'}>
            <ComputerCard open={() => open().includes('computer')} />
          </Section>
        </Show>
        <Show when={recall().length > 0 || recallTrace()}>
          <Section id="recall" title="Recall" count={recall().length}>
            <Show when={recallTrace()}>{trace => (
              <div class="side-card-recall-trace">
                <p class="side-card-recall-why">{recallTraceWords(trace())}</p>
                <ul class="side-card-recall-trace-list" aria-label="Recall trace">
                  <For each={trace().candidates}>{row => (
                    <li class={row.kept ? 'kept' : 'cut'} title={row.path}><span>{row.subject || row.path}</span> <em>{row.kept ? 'used' : 'left out'}: {row.why}</em></li>
                  )}</For>
                </ul>
              </div>
            )}</Show>
            <ul class="side-card-recall">
              <For each={recall()}>{item => {
                const [expanded, setExpanded] = createSignal(false);
                return (
                <li class="side-card-recall-item">
                  <button type="button" class="side-card-recall-toggle" aria-expanded={expanded()} onClick={() => setExpanded(v => !v)}>
                    <Icon name={expanded() ? 'chevronDown' : 'chevronRight'} size={14} /><strong>{item.subject}</strong>
                  </button>
                  <Show when={expanded()}>
                  <p>{item.statement}</p>
                  <code title={item.path}>{item.path}</code>
                  <Show when={item.lifecycle}>{lifecycle => <span class="side-card-recall-lifecycle">{lifecycle()}</span>}</Show>
                  <div class="side-card-recall-actions">
                    <button type="button" class="btn ghost sm" onClick={() => appendToChatDraft(recallFeedbackPrompt(item, 'notRelevant'))}>Not relevant</button>
                    <button type="button" class="btn ghost sm" onClick={() => appendToChatDraft(recallFeedbackPrompt(item, 'outdated'))}>Outdated</button>
                  </div>
                  </Show>
                </li>
              ); }}</For>
            </ul>
          </Section>
        </Show>
      </div>
    </aside>
  );
}
