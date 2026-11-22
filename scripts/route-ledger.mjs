#!/usr/bin/env node
// Writes docs/ROUTE_LEDGER.md and docs/ACTIONS.md from src/routeLedger.ts and
// src/actionDefs.ts (consolidation D, 2026-10-13; plan completion L4). Both
// modules are pure data, so Node's own type stripping loads them;
// src/routeLedger.test.ts fails while either doc is stale.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const { renderRouteLedger, renderActions } = await import('../src/routeLedger.ts');
for (const [file, text] of [['ROUTE_LEDGER.md', renderRouteLedger()], ['ACTIONS.md', renderActions()]]) {
  const out = fileURLToPath(new URL(`../docs/${file}`, import.meta.url));
  writeFileSync(out, text);
  console.log(`route-ledger: wrote ${out}`);
}
