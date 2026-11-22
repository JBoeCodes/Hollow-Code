// The side panel's card and New tab (overnight 11-12 C4 and C5): what the
// card lists under Outputs, Subagents, Sources and Recall, and which Tools a New tab
// offers. Pure, unit-tested; `components/SidePanelCard.tsx` and
// `components/PanelNewTab.tsx` draw them.
import { countsText } from './subagentsModel';
import { chipFolders, chipLabel, chipName, fileKind, type FileTone } from './fileChip';
import type { IconName } from './icons';
import type { SubagentsView, WorkFile } from './generated/ipc';

/** How many agent glyphs the card's Subagents line shows. */
export const CARD_GLYPHS = 4;
/** How many outputs and sources the card lists before View all. */
export const CARD_FILES = 3;

export const CARD_SECTIONS = ['outputs', 'subagents', 'sources', 'recall', 'computer', 'servers', 'automations'] as const;
export type CardSection = typeof CARD_SECTIONS[number];
export const CARD_SECTIONS_KEY = 'hollow.sideCard.sections.v1';
/** Only explicit saved open sections override the collapsed default. */
export function parseCardSections(raw: string | null): CardSection[] {
  try {
    const value: unknown = JSON.parse(raw ?? '[]');
    return Array.isArray(value) ? CARD_SECTIONS.filter(id => value.includes(id)) : [];
  } catch { return []; }
}

/** One line saying why a turn's recall was what it was (RC1): the silence rule
 *  that cut it, that the chat's session already holds it, or how many
 *  candidates were used. */
export function recallTraceWords(trace: { silence?: string; alreadyGiven?: boolean; candidates: Array<{ kept: boolean }> }): string {
  if (trace.silence) return `Recall stayed quiet: ${trace.silence}.`;
  if (!trace.candidates.length) return 'Nothing in memory matched this request.';
  const used = trace.candidates.filter(c => c.kept).length;
  return trace.alreadyGiven ? `${used} used; this chat already holds them, so they are not sent again.` : `${used} used.`;
}

/** Draft a correction for review without changing trusted context directly. */
export function recallFeedbackPrompt(item: { statement: string; path: string }, action: 'notRelevant' | 'outdated'): string {
  const reason = action === 'outdated'
    ? 'is outdated and needs a replacement'
    : 'was not relevant to this request and should be recalled more narrowly';
  return `Teach Hollow: the accepted learning "${item.statement}" from ${item.path} ${reason}. Ask me for the corrected durable statement if needed, then create only a review proposal; do not change trusted context directly.`;
}

type CardView = Pick<SubagentsView, 'running' | 'queued' | 'doneCount' | 'active' | 'done'>;

/** One glyph per subagent, up to four: the active ones first, then the
 *  newest done (the same agent can repeat, one mark per subagent). */
export function cardGlyphs(view: Pick<SubagentsView, 'active' | 'done'> | null | undefined, max = CARD_GLYPHS): string[] {
  return [...(view?.active ?? []), ...(view?.done ?? [])].slice(0, max).map(row => row.agent);
}

/** The card's Subagents line: "4 done", "2 running, 5 done", "2 earlier
 *  chats" when a lead's or the COO's other chats are all it has (L8: the
 *  button is their way in, as the header chip was before #1492), or, before
 *  the chat has started any, "None yet" ("Reading…" until the first read). */
export function cardSubagentWords(view: (CardView & { earlier?: readonly unknown[] }) | null | undefined): string {
  if (!view) return 'Reading…';
  const counts = countsText(view);
  if (counts) return counts;
  const n = view.earlier?.length ?? 0;
  return n ? `${n} earlier chat${n === 1 ? '' : 's'}` : 'None yet';
}

const newest = (a: WorkFile, b: WorkFile) => (b.modified ?? '').localeCompare(a.modified ?? '') || a.name.localeCompare(b.name);

/** The workspace's `outputs/` files, newest first, up to `max`. */
export function cardOutputs(files: readonly WorkFile[] | null | undefined, max = CARD_FILES): WorkFile[] {
  return (files ?? []).filter(f => f.kind === 'output').sort(newest).slice(0, max);
}

/** The workspace's sources, newest first, up to `max`. */
export function cardSources(files: readonly WorkFile[] | null | undefined, max = CARD_FILES): WorkFile[] {
  return (files ?? []).filter(f => f.kind === 'source').sort(newest).slice(0, max);
}

/** One Outputs or Sources row (overnight 11-12 C8, Alex's 11-10 file
 *  chip decision): the file's name with its type icon and colour
 *  (`fileChip.ts`, as a reply's chips), the full path for the tooltip, the
 *  accessible name and a copy. */
export function cardFileRow(path: string, folder = ''): { path: string; name: string; folder: string; icon: IconName; tone: FileTone; label: string } {
  const { icon, tone } = fileKind(path);
  return { path, name: chipName(path).name, folder, icon, tone, label: chipLabel(path) };
}

/** A list's rows, each with the folder words that tell it from a same-name
 *  file in that list (`chipFolders`, as reply chips; '' when its name is
 *  the only one). */
export function cardFileRows(paths: readonly string[]): ReturnType<typeof cardFileRow>[] {
  const folders = chipFolders(paths);
  return paths.map((p, i) => cardFileRow(p, folders[i]));
}

/** Where the card's Sources "View all" goes: SOURCES.md when the project
 *  has one, else WORKSPACE.md (vault-relative). */
export function viewAllSourcesPath(workspacePath: string, files: readonly { path: string }[] | null | undefined): string {
  const sources = `${workspacePath}/SOURCES.md`;
  return (files ?? []).some(f => f.path === sources) ? sources : `${workspacePath}/WORKSPACE.md`;
}

/** One tile on a New tab's Tools grid. */
export interface PanelTool {
  id: 'files' | 'browser' | 'subagents' | 'terminal' | 'changes';
  label: string;
  detail: string;
  /** Why it can't open here; null when it can. */
  unavailable: string | null;
}

/** The New tab's Tools: Files (a project's notes), Browser, Subagents (the
 *  chat on screen's), Terminal (parity 7.2) and, for a build workspace, Changes. */
export function panelTools(input: { project: boolean; chat: boolean; build: boolean }): PanelTool[] {
  const tools: PanelTool[] = [
    { id: 'files', label: 'Files', detail: 'WORKSPACE.md, PLAN.md, SOURCES.md and AGENTS.md', unavailable: input.project ? null : 'Open a project first: Chats has no project files.' },
    { id: 'browser', label: 'Browser', detail: 'Search or enter a URL', unavailable: null },
    { id: 'subagents', label: 'Subagents', detail: 'What this chat started', unavailable: input.chat ? null : 'Send a message first: subagents belong to a chat.' },
    { id: 'terminal', label: 'Terminal', detail: 'A shell in this chat’s folder, where its mode allows one', unavailable: null },
  ];
  if (input.build) tools.push({ id: 'changes', label: 'Changes', detail: 'Changed files, commit and publish', unavailable: null });
  return tools;
}

/** One row of the card's Open in menu (`open_in_targets`: Finder, then the
 *  code editors installed on this Mac). */
export interface OpenInTarget { id: string; label: string }

/** The Open in menu's rows: Finder always first (a list without it, or a
 *  failed read, still offers it), every other id once, in the backend's order. */
export function openInRows(targets: readonly OpenInTarget[] | null | undefined): OpenInTarget[] {
  const rows: OpenInTarget[] = [{ id: 'finder', label: 'Finder' }];
  for (const t of targets ?? []) if (t.id && t.label && !rows.some(r => r.id === t.id)) rows.push({ id: t.id, label: t.label });
  return rows;
}

/** The card's machine row: this Mac's name, else a plain "This Mac". */
export function cardMachineName(name: string | null | undefined): string {
  return name?.trim() || 'This Mac';
}

