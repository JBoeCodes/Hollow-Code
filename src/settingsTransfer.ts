// Settings export/import (overnight F7): the words the flow shows. Pure, so
// the node vitest suite loads it; the dialogs and calls are in
// store/settingsTransfer.ts and the Rust side is settings_transfer.rs.
import type { SettingsExported, SettingsImportApplied, SettingsImportPreview } from './types';

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/** The default file name: `hollow-settings-2026-10-10.json`. */
export function exportFileName(now: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `hollow-settings-${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}.json`;
}

/** After an export: how many settings went where, and what was left out. */
export function exportedText(r: SettingsExported): string {
  const skipped = r.skipped.length ? ` (${plural(r.skipped.length, 'setting')} couldn’t be read and were left out)` : '';
  return `Exported ${plural(r.count, 'setting')} to ${r.path}. Secrets and sign-ins are never included.${skipped}`;
}

/** The confirm dialog's body: every change old → new, then what is
 *  ignored and why. */
export function previewBody(p: SettingsImportPreview): string {
  // What agents may do, what runs or spends on its own, accounts: first,
  // with what each one is.
  const careful = p.changes.filter(c => c.asks);
  const plain = p.changes.filter(c => !c.asks);
  const lines = [
    ...(careful.length ? ['These change what agents may do, what runs on its own, or accounts:', ...careful.map(c => `• ${c.key}: ${c.old} → ${c.new} (${c.summary.replace(/\.$/, '')})`), ''] : []),
    ...plain.map(c => `• ${c.key}: ${c.old} → ${c.new}`),
  ];
  if (lines[lines.length - 1] === '') lines.pop();
  const head = `${plural(p.changes.length, 'setting')} will change${p.unchanged ? `; ${p.unchanged} already match` : ''}${p.exportedAt ? ` (file exported ${p.exportedAt.slice(0, 10)})` : ''}.`;
  const ignored = p.ignored.length
    ? ['', `Left alone (${p.ignored.length}):`, ...p.ignored.map(i => `• ${i.key}: ${i.why}`)]
    : [];
  return [head, '', ...lines, ...ignored].join('\n');
}

/** When the file changes nothing. */
export function nothingToImportText(p: SettingsImportPreview): string {
  const ignored = p.ignored.length ? ` ${plural(p.ignored.length, 'entry', 'entries')} left alone.` : '';
  return `Nothing to import: every setting in the file already matches.${ignored}`;
}

/** After an import: what changed, and what failed with its reason. */
export function appliedText(r: SettingsImportApplied): string {
  if (!r.failed.length) return `Imported: ${plural(r.applied, 'setting')} changed.`;
  const first = r.failed[0];
  return `× Imported ${plural(r.applied, 'setting')}; ${plural(r.failed.length, 'setting')} failed (${first.key}: ${first.why}).`;
}
