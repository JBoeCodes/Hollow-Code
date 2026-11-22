// Frontend metadata for the semantic vault index. The backend
// (`vault_index.rs`) owns the authoritative classification (kind / surface /
// editable / hidden / trust); this module owns how those kinds present in the
// UI — their label + icon, and which **lens** they group under in the Vault
// tab's left rail. Keep the `VaultKind` union (in types.ts) and the `KIND_*`
// Rust constants in lockstep with the tables here.

import type { VaultItem, VaultKind, VaultTrust } from './types';
import type { IconName } from './icons';
import { ICON_MEANING } from './iconMeaning';
import { isWorkspaceVaultItem } from './workspaceContent';

/** Per-kind display metadata. `icon` is an icons.tsx name. */
export const KIND_META: Record<Exclude<VaultKind, 'email' | 'chat'>, { label: string; icon: IconName }> = {
  task:      { label: 'Task',      icon: 'checkSquare' },
  project:   { label: 'Project',   icon: 'folder' },
  meeting:   { label: 'Meeting',   icon: 'calendar' },
  recording: { label: 'Recording', icon: 'book' },
  workflow:  { label: 'Automation', icon: 'zap' },
  briefing:  { label: 'Briefing',  icon: 'fileText' },
  person:    { label: 'Person',    icon: 'users' },
  domain:    { label: 'Domain',    icon: 'globe' },
  daily:     { label: 'Daily',     icon: 'calendar' },
  weekly:    { label: 'Weekly',    icon: 'calendar' },
  goal:      { label: 'Goal',      icon: 'flag' },
  commitment:{ label: 'Commitment',icon: 'checkSquare' },
  learning:  { label: 'Learning',  icon: ICON_MEANING.memory },
  note:      { label: 'Note',      icon: 'fileText' },
  source:    { label: 'Source',    icon: 'book' },
  operator:  { label: 'Operator',  icon: 'users' },
  capture:   { label: 'Capture',   icon: 'filePlus' },
  unfiled:   { label: 'Unfiled',   icon: 'alert' },
};

export function kindLabel(kind: VaultKind): string {
  return kind === 'email' || kind === 'chat' ? 'Note' : KIND_META[kind]?.label ?? kind;
}

/** Human label + tone for the trust/read-only badge in the note header. */
export const TRUST_META: Record<VaultTrust, { label: string; readOnlyNote: string }> = {
  authored:  { label: 'Authored',  readOnlyNote: '' },
  synced:    { label: 'Synced',    readOnlyNote: 'Synced from an external source — edits here are overwritten on the next sync.' },
  generated: { label: 'Generated', readOnlyNote: 'Written by an automation; its next run writes it again.' },
  imported:  { label: 'Imported',  readOnlyNote: '' },
};

export type LensId =
  | 'recent' | 'daily' | 'tasks' | 'projects' | 'people' | 'domains'
  | 'notes' | 'sources' | 'meetings' | 'generated'
  | 'cleanup' | 'history' | 'all';

export interface VaultLens {
  id: LensId;
  label: string;
  icon: IconName;
  /** Kinds this lens collects. Omitted for the special lenses (recent /
   *  cleanup / all), whose membership is computed by id in `matchesLens`. */
  kinds?: VaultKind[];
  /** Shown when the lens is empty. */
  hint: string;
  /** Tooltip on the lens chip — explains what the lens collects, since the
   *  semantic-classification model isn't otherwise visible in the UI. */
  desc: string;
}

/** The lens rail, in display order. The default browse experience leads with
 *  curated, current knowledge; connected calendar/recording notes and
 *  generated artifacts get their own opt-in
 *  lenses instead of flooding the tree. `all` keeps the raw folder tree. */
export const LENSES: VaultLens[] = [
  { id: 'recent',    label: 'Recent',    icon: 'history',    hint: 'No recent notes.',
    desc: 'Everything current, newest first — synced mirrors stay out of the way' },
  { id: 'daily',     label: 'Daily',     icon: 'calendar',    kinds: ['daily', 'weekly'], hint: 'No daily or weekly notes yet.',
    desc: 'Daily and weekly notes (your Plan/Close ritual output)' },
  { id: 'tasks',     label: 'Tasks',     icon: 'checkSquare', kinds: ['task'], hint: 'No task notes yet.',
    desc: 'Task notes with an optional structured task editor' },
  { id: 'projects',  label: 'Projects',  icon: 'folder',      kinds: ['project'], hint: 'No project notes yet.',
    desc: 'Project notes with an optional structured project editor' },
  { id: 'people',    label: 'People',    icon: 'users',       kinds: ['person'], hint: 'No people notes yet.',
    desc: 'Stakeholder notes (people/) — snapshots, comms rules, sensitivities' },
  { id: 'domains',   label: 'Domains',   icon: 'globe',       kinds: ['domain'], hint: 'No domain knowledge notes yet.',
    desc: 'Durable domain knowledge — the notes that explain how things work' },
  { id: 'notes',     label: 'Notes',     icon: 'fileText',    kinds: ['note', 'operator', 'capture'], hint: 'No free notes yet.',
    desc: 'Free-form notes, operator profile, and quick captures' },
  { id: 'sources',   label: 'Sources',   icon: 'book',        kinds: ['source'], hint: 'No imported sources yet.',
    desc: 'Imported articles, files and links (Import a source writes here)' },
  { id: 'meetings',  label: 'Meetings',  icon: 'calendar',    kinds: ['meeting', 'recording'], hint: 'No meetings or recordings.',
    desc: 'Calendar meeting notes and Plaud recordings (synced, read-first)' },
  { id: 'generated', label: 'Generated', icon: 'zap',         kinds: ['briefing', 'workflow'], hint: 'No briefings or automation results yet.',
    desc: 'Produced by agents: briefings and automation results' },
  { id: 'cleanup',   label: 'Cleanup',   icon: 'alert',       hint: 'Nothing to clean up — no unfiled or untyped notes.',
    desc: 'Drift queue: unfiled or untyped notes that need a home or frontmatter' },
  { id: 'history',   label: 'History',   icon: 'refresh',     hint: 'Automated-write history opens in the main pane.',
    desc: 'Run-scoped automated-write history with guarded, reversible restore' },
  { id: 'all',       label: 'All Files', icon: 'folder',      hint: '',
    desc: 'The raw folder tree — every file, no classification, drag-drop moves' },
];

export const LENS_BY_ID: Record<LensId, VaultLens> =
  Object.fromEntries(LENSES.map(l => [l.id, l])) as Record<LensId, VaultLens>;

/** Whether a classified item belongs in the given lens.
 *  - `recent`  → everything not hidden by default (the curated browse view).
 *  - `cleanup` → anything the classifier flagged with a drift reason.
 *  - `all`     → not used here (the All Files lens renders the raw tree).
 *  - others    → membership by kind. */
export function matchesLens(item: VaultItem, lensId: LensId): boolean {
  if (!isWorkspaceVaultItem(item)) return false;
  switch (lensId) {
    case 'recent':  return !item.hidden;
    case 'cleanup': return item.drift != null;
    case 'history': return false;
    case 'all':     return true;
    default: {
      const lens = LENS_BY_ID[lensId];
      return !!lens?.kinds?.includes(item.kind);
    }
  }
}
