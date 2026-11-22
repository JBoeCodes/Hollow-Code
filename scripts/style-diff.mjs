#!/usr/bin/env node
/**
 * Compares two computed-style captures written by
 * `FRONTIER_STYLES=1 FRONTIER_LABEL=<label> … tests/review/frontier-capture.spec.ts`.
 *
 *   node scripts/style-diff.mjs <labelA> <labelB> [--detail N] [--prop name] [--only variant/surface]
 *
 * Reads test-results/frontier/<label>/<variant>/<surface>.styles.json (or
 * $STYLE_DIFF_ROOT/<label>/…; Playwright empties test-results/ on every run,
 * so copy a capture set out before the next one) and
 * prints, per surface, how many elements changed which properties, then the
 * first N concrete differences. Use it to prove a stylesheet refactor (moving
 * tokens, layering the cascade, folding files) is style-neutral, and to see
 * exactly what an intentional change touched.
 */
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const [a, b, ...rest] = process.argv.slice(2);
if (!a || !b) { console.error('usage: style-diff.mjs <labelA> <labelB> [--detail N] [--prop name]'); process.exit(2); }
const detail = Number(rest[rest.indexOf('--detail') + 1]) || (rest.includes('--detail') ? 20 : 0);
const onlyProp = rest.includes('--prop') ? rest[rest.indexOf('--prop') + 1] : null;
const onlySurface = rest.includes('--only') ? rest[rest.indexOf('--only') + 1] : null;
const root = process.env.STYLE_DIFF_ROOT ?? 'test-results/frontier';
const dirA = join(root, a), dirB = join(root, b);
let total = 0;
for (const variant of readdirSync(dirA)) {
  if (!existsSync(join(dirB, variant))) continue;
  for (const file of readdirSync(join(dirA, variant)).filter(f => f.endsWith('.styles.json'))) {
    if (onlySurface && !`${variant}/${file}`.includes(onlySurface)) continue;
    const fb = join(dirB, variant, file);
    if (!existsSync(fb)) { console.log(`${variant}/${file}: missing in ${b}`); continue; }
    const A = JSON.parse(readFileSync(join(dirA, variant, file), 'utf8'));
    const B = JSON.parse(readFileSync(fb, 'utf8'));
    const byProp = new Map();
    const diffs = [];
    let onlyA = 0, onlyB = 0;
    for (const [path, recA] of Object.entries(A)) {
      const recB = B[path];
      if (!recB) { onlyA++; continue; }
      for (const prop of new Set([...Object.keys(recA), ...Object.keys(recB)])) {
        if (onlyProp && !prop.includes(onlyProp)) continue;
        if (recA[prop] !== recB[prop]) {
          byProp.set(prop, (byProp.get(prop) ?? 0) + 1);
          diffs.push({ path, prop, from: recA[prop], to: recB[prop] });
        }
      }
    }
    for (const path of Object.keys(B)) if (!A[path]) onlyB++;
    if (!diffs.length && !onlyA && !onlyB) continue;
    total += diffs.length;
    const props = [...byProp].sort((x, y) => y[1] - x[1]).map(([p, n]) => `${p}×${n}`).join(' ');
    console.log(`${variant}/${file.replace('.styles.json', '')}: ${diffs.length} diffs${onlyA || onlyB ? ` (elements only in ${a}: ${onlyA}, only in ${b}: ${onlyB})` : ''}  ${props}`);
    for (const d of diffs.slice(0, detail)) {
      const tail = d.path.split('>').slice(-3).join('>');
      console.log(`    ${tail}\n      ${d.prop}: ${String(d.from).slice(0, 90)}  →  ${String(d.to).slice(0, 90)}`);
    }
  }
}
console.log(total ? `\n${total} property differences` : 'no differences');
