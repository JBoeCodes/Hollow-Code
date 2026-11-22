/**
 * Pure helpers for Settings, Personalization (the profile builder panel,
 * `settings/AboutYouPanel.tsx`; backend `profile_builder.rs`). No store or
 * Tauri imports, so the node vitest suite can load it.
 */
import type { ProfileBuilderView, ProfileFileView, ProfileSource, ProfileSuggestion } from './types';
import { formatRelative } from './formatTime';

/** Re-read what agents know on opening the page when it's older than this. */
export const AUTO_SCAN_AFTER_MS = 3 * 24 * 60 * 60 * 1000;

const SOURCE_NAMES: Record<string, string> = {
  'claude-instructions': 'Claude Code instructions',
  'claude-memory': 'Claude memories',
  'codex-instructions': 'Codex instructions',
  'gemini-instructions': 'Gemini instructions',
  account: 'Claude account',
  'hollow-memory': 'Hollow memories',
  conversations: 'your recent requests',
};

/** "from Claude memories" for a suggestion's source id. */
export function sourceName(id: string): string {
  return SOURCE_NAMES[id] ?? 'your agents';
}

/** A source chip: "12 Claude memories", "Codex instructions", "40 recent requests". */
export function sourceChip(source: ProfileSource): string {
  switch (source.id) {
    case 'claude-memory': return `${source.count} Claude ${source.count === 1 ? 'memory' : 'memories'}`;
    case 'hollow-memory': return `${source.count} Hollow ${source.count === 1 ? 'memory' : 'memories'}`;
    case 'conversations': return `${source.count} recent ${source.count === 1 ? 'request' : 'requests'}`;
    default: return source.label;
  }
}

/** Where a fact will land: "Communication › Always", "Identity · Name". */
export function targetLabel(s: { file: string; section: string | null; label?: string | null }): string {
  const title = s.file.replace(/\.md$/, '');
  if (s.label) return `${title} · ${s.label}`;
  return s.section ? `${title} › ${s.section}` : title;
}

/** Suggestions grouped by note, in the profile's file order. */
export function groupSuggestions(
  suggestions: ProfileSuggestion[],
  order: string[],
): { file: string; title: string; items: ProfileSuggestion[] }[] {
  const byFile = new Map<string, ProfileSuggestion[]>();
  for (const s of suggestions) {
    const list = byFile.get(s.file) ?? [];
    list.push(s);
    byFile.set(s.file, list);
  }
  const files = [...order.filter(f => byFile.has(f)), ...[...byFile.keys()].filter(f => !order.includes(f))];
  return files.map(file => ({ file, title: file.replace(/\.md$/, ''), items: byFile.get(file)! }));
}

/** Compact relative time or an absolute date, or null when never. */
export function lastLooked(scannedAt: string | null, now: number = Date.now()): string | null {
  if (!scannedAt) return null;
  const at = Date.parse(scannedAt);
  if (Number.isNaN(at)) return null;
  return formatRelative(at, now);
}

/**
 * Open the page and read on its own: never read before, or the last read is
 * older than {@link AUTO_SCAN_AFTER_MS}. Only with Keep learning on, like the
 * background timer (`profile_builder::background_due`): off says "Only when
 * you ask" (P0-11). Not while a read runs or when the last one failed (the
 * user retries with Look again).
 */
export function shouldAutoScan(view: ProfileBuilderView, now: number = Date.now()): boolean {
  if (!view.autoLearn || view.scanning || view.lastError) return false;
  if (!view.scannedAt) return true;
  const at = Date.parse(view.scannedAt);
  return Number.isNaN(at) || now - at > AUTO_SCAN_AFTER_MS;
}

/** Filled details and blanks across the profile notes. */
export function completeness(profile: ProfileFileView[]): { filled: number; blanks: number; missing: number } {
  let filled = 0;
  let blanks = 0;
  let missing = 0;
  for (const f of profile) {
    filled += f.items.length;
    blanks += f.blanks;
    if (!f.exists) missing += 1;
  }
  return { filled, blanks, missing };
}

/** The one-line headline over the panel. */
export function headline(view: ProfileBuilderView): string {
  const { filled, blanks } = completeness(view.profile);
  const pending = view.suggestions.length + view.questions.length;
  if (filled === 0 && pending === 0) return 'Your agents don’t know much about you yet.';
  const parts = [`${filled} ${filled === 1 ? 'detail' : 'details'} on file`];
  if (blanks) parts.push(`${blanks} ${blanks === 1 ? 'blank' : 'blanks'} to fill`);
  if (view.suggestions.length) parts.push(`${view.suggestions.length} ready to add`);
  return parts.join(' · ');
}
