#!/usr/bin/env node
// Window widths are deliberately few. Unnamed local container queries measure
// a pane/card, not the window, and are outside this window-layout contract.
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const STYLES = fileURLToPath(new URL('../src/styles/', import.meta.url));
export function windowBreakpoints(files = readdirSync(STYLES).filter(f => f.endsWith('.css')).map(name => ({ name, text: readFileSync(join(STYLES, name), 'utf8') }))) {
  const widths = new Set();
  const problems = [];
  for (const { name, text } of files) {
    if (name === 'tokens.css') for (const [tier, em] of [['compact', 53.75], ['medium', 68.75]]) {
      if (Number(text.match(new RegExp(`--window-${tier}-em:\\s*([\\d.]+)\\s*;`))?.[1]) !== em)
        problems.push(`tokens.css: --window-${tier}-em must match the shared ${em}em ${tier} query`);
    }
    for (const match of text.matchAll(/@(media|container\s+hollow-window)\s*([^{}]*\bwidth\s*[^{}]*)\{/g)) {
      const line = text.slice(0, match.index).split('\n').length;
      const query = match[2].trim();
      widths.add(query.replace(/\s+/g, ''));
      if (match[1] === 'media') {
        // The sole narrower test-host rule is explicit at each block, not
        // an allowlist for a whole file that could hide new window widths.
        if (!/^\(max-width:\s*600px\)$/.test(query) || !/\/\* Narrow window: 390px Playwright host\. \*\/\s*$/.test(text.slice(0, match.index)))
          problems.push(`${name}:${line}: unnamed window breakpoint ${query}`);
      } else if (!/^\((?:max-width:\s*(?:53\.75|68\.75)em|width\s*>\s*68\.75em)\)$/.test(query)) {
        problems.push(`${name}:${line}: unknown shared window breakpoint ${query}`);
      }
    }
  }
  if (widths.size > 4) problems.push(`window queries use ${widths.size} distinct widths, maximum 4`);
  return { widths: [...widths].sort(), problems };
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { widths, problems } = windowBreakpoints();
  console.log(`Window queries: ${widths.join(', ')}`);
  if (problems.length) { console.error(problems.join('\n')); process.exit(1); }
}
