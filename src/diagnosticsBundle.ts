// "Save diagnostics…" (overnight C6): the words around the redacted zip that
// `save_diagnostics` writes. Pure, so the unit tests import it directly.
import type { DiagnosticsSaved } from './types';

/** The footer line after a save: where it went and how big it is. */
export function savedDiagnosticsText(saved: DiagnosticsSaved): string {
  const kb = Math.max(1, Math.round(saved.bytes / 1024));
  return `Saved diagnostics (${saved.entries} files, ${kb} KB) to ${saved.path}. Nothing was sent.`;
}

/** Hint under the button: what goes in, what never does. */
export const DIAGNOSTICS_HINT =
  'A zip of versions, health lines, run records (status and timings only) and install-log tails, '
  + 'with secrets, email addresses and your paths masked. Debug logs are listed by name only, and note text '
  + 'is never included. Saved on this Mac to a folder you choose; Hollow never sends it.';
