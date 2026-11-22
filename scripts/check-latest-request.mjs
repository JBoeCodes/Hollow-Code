#!/usr/bin/env node
// Plan L3 X21a (overnight 2026-10-15 H8): an async view drops a stale response
// with `latestRequest()` (src/latestRequest.ts), not a hand-rolled counter
// (`const mine = ++gen; … if (mine !== gen) return`). The views below were
// migrated; a counter's new value assigned in one of them (`= ++gen`,
// `= gen++`, `= (gen += 1)`) is a counter coming back, so this fails. A
// heuristic over the source text: a `++x` that is an id rather than a guard
// (`${++editorSeq}`, `() => ++seq`) has no assignment before it and passes,
// as does a counter bumped on its own (`saveEpoch += 1`). X21b (H9)
// added the stores and the three pure factories; the counters they keep on
// purpose (ids and ordered sequence numbers: `nextChatAttachmentId`,
// `artifactSeq`, `gitFreshVersion`, feedback's `saveEpoch`) are written so
// they pass.
//
//   node scripts/check-latest-request.mjs        (also run by npm run test:scripts)
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');

/** The views (X21a) and stores (X21b) migrated to `latestRequest()`,
 *  relative to the repository root. */
export const MIGRATED = [
  'src/components/BuildChanges.tsx',
  'src/components/CanvasGallery.tsx',
  'src/components/CommandPalette.tsx',
  'src/components/ConnectedProjects.tsx',
  'src/components/ConversationHistory.tsx',
  'src/components/SourceRefresh.tsx',
  'src/components/WorkFileEditor.tsx',
  // S5 removed the Import window (QuickIngest.tsx); the Add sheet that
  // replaced it drops stale openings the same way.
  'src/modals/AddModal.tsx',
  'src/pages/Library.tsx',
  'src/settings/CompanionDevicesPanel.tsx',
  // X21b: the stores and the pure factories. Votes left the window (overnight
  // 11-08 S5) and their store went in the 11-10 sweep.
  'src/needsYou.ts',
  'src/paletteSearch.ts',
  'src/settings/controllers/settingsNavigation.ts',
  'src/pages/Skills.tsx',
  'src/store/chatRuntime.ts',
  'src/store/mainAgent.ts',
  'src/store/prepPack.ts',
  'src/store/runQueue.ts',
  'src/store/triage.ts',
  'src/store/workspaces.ts',
];

// An assignment (`=`, not `==`, `+=`, `=>` …) of `++x`, `x++` or `x += 1`.
const GUARD = /(?<![=!<>+\-*/%&|^?])=(?![=>])\s*\(?\s*(?:\+\+\s*[A-Za-z_$][\w$]*|[A-Za-z_$][\w$]*\s*(?:\+\+|\+=\s*1\b))/g;

/** Every counter guard (see GUARD) in `files`, as `path:line: text`. */
export function handRolledGuards(files = MIGRATED, read = f => readFileSync(join(ROOT, f), 'utf8')) {
  const out = [];
  for (const f of files) {
    read(f).split('\n').forEach((line, i) => {
      for (const m of line.matchAll(GUARD)) out.push(`${f}:${i + 1}: ${m[0]}`);
    });
  }
  return out;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const bad = handRolledGuards();
  if (bad.length) { console.error(`Use latestRequest() (src/latestRequest.ts), not a hand-rolled request counter:\n  ${bad.join('\n  ')}`); process.exit(1); }
  console.log(`latest request: ${MIGRATED.length} migrated files use latestRequest(), no hand-rolled counters`);
}
