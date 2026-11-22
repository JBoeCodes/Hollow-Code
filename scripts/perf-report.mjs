#!/usr/bin/env node
// Five asks B4: one repeatable performance report. Prints JSON to stdout and
// writes nothing outside $TMPDIR. Run it in the foreground after a build:
//   npm run build && node scripts/perf-report.mjs            # bundle + launch set
//   node scripts/perf-report.mjs --modules                     # + what fills the launch chunks
// `--modules` runs one in-memory Vite build (write: false), so it counts as a
// toolchain slot; the default only reads dist/.
//
// The browser half is scripts/perf-mock-budgets.mjs (needs build:mock and one
// vite preview server). `npm run bundle:budget` turns this report's numbers
// into failing caps.
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';

const KB = 1024;
const kb = n => +(n / KB).toFixed(1);
const IMAGE = /\.(png|jpe?g|gif|webp|avif|svg)$/i;
const FONT = /\.(woff2?|ttf|otf)$/i;

/** Relative static imports of one built ES module (`import{a}from"./x.js"`,
 * `import"./x.js"`, `export*from"./x.js"`); `import("./x.js")` is lazy and
 * deliberately not matched. */
export function staticImports(code) {
  const out = new Set();
  const re = /(?:^|[;\s}])(?:import|export)\s*(?:[\w$*{}\s,]+?\s*from\s*)?["'](\.\/[^"']+\.js)["']/g;
  for (const m of code.matchAll(re)) out.add(m[1].slice(2));
  return [...out];
}

/** Files a lazy `import("./<prefix>…js")` in `code` preloads with it: Vite
 * wraps it as `import("./X.js")…,__vite__mapDeps([i, …])`, indices into the
 * chunk's `m.f=[…]` list. */
export function preloadGroup(code, prefix) {
  const list = code.match(/m\.f=\[([^\]]*)\]/);
  const files = list ? [...list[1].matchAll(/"assets\/([^"]+)"/g)].map(m => m[1]) : [];
  const out = new Set();
  const re = new RegExp(`import\\("\\./(${prefix}[^"]+\\.js)"\\)[^;]{0,80}?__vite__mapDeps\\(\\[([\\d,]*)\\]\\)`, 'g');
  for (const m of code.matchAll(re)) {
    out.add(m[1]);
    for (const i of m[2].split(',').filter(Boolean)) if (files[+i]) out.add(files[+i]);
  }
  return [...out];
}

/** What a cold launch of the main window loads: the HTML entry, the `App`
 * chunk main.tsx imports straight away with its preload group, every static
 * import of those, and the linked stylesheets. */
export function launchSet(indexHtml, readChunk) {
  const entries = [...indexHtml.matchAll(/<script[^>]+src="\/assets\/([^"]+\.js)"/g)].map(m => m[1]);
  const css = new Set([...indexHtml.matchAll(/<link[^>]+rel="stylesheet"[^>]+href="\/assets\/([^"]+\.css)"/g)].map(m => m[1]));
  if (!entries.length) throw new Error('launch set: index.html names no /assets/*.js entry');
  const roots = [...entries];
  for (const e of entries) {
    const code = readChunk(e);
    // The App import itself, even when Vite gives it no preload group.
    for (const m of code.matchAll(/import\("\.\/(App-[^"]+\.js)"\)/g)) roots.push(m[1]);
    for (const f of preloadGroup(code, 'App-')) (f.endsWith('.css') ? css.add(f) : roots.push(f));
  }
  const seen = new Set();
  const stack = [...roots];
  while (stack.length) {
    const name = stack.pop();
    if (seen.has(name)) continue;
    seen.add(name);
    for (const dep of staticImports(readChunk(name))) stack.push(dep);
  }
  if (![...seen].some(f => f.startsWith('App-'))) throw new Error('launch set: the entry imports no App-*.js chunk');
  return { js: [...seen], css: [...css] };
}

function bundleReport(dist) {
  const dir = join(dist, 'assets');
  const files = readdirSync(dir).map(name => {
    const buf = readFileSync(join(dir, name));
    const compressible = /\.(js|css|svg)$/.test(name);
    return { name, raw: buf.length, gzip: compressible ? gzipSync(buf, { level: 9 }).length : buf.length };
  }).sort((a, b) => b.raw - a.raw);
  const by = re => files.filter(f => re.test(f.name));
  const sum = (list, k) => list.reduce((n, f) => n + f[k], 0);
  const read = name => readFileSync(join(dir, name), 'utf8');
  const launch = launchSet(readFileSync(join(dist, 'index.html'), 'utf8'), read);
  const launchFiles = [...launch.js, ...launch.css].map(n => files.find(f => f.name === n)).filter(Boolean);
  const launchCode = launch.js.map(read).join('\n');
  // Images a launch chunk names. Not all of them paint at launch (a page has to
  // render first), but each is reachable without a lazy import.
  const launchImages = by(IMAGE).filter(f => launchCode.includes(f.name));
  return {
    totals: {
      js: { count: by(/\.js$/).length, rawKB: kb(sum(by(/\.js$/), 'raw')), gzipKB: kb(sum(by(/\.js$/), 'gzip')) },
      css: { count: by(/\.css$/).length, rawKB: kb(sum(by(/\.css$/), 'raw')), gzipKB: kb(sum(by(/\.css$/), 'gzip')) },
      images: { count: by(IMAGE).length, rawKB: kb(sum(by(IMAGE), 'raw')) },
      fonts: { count: by(FONT).length, rawKB: kb(sum(by(FONT), 'raw')) },
    },
    launch: {
      js: launch.js.map(n => { const f = files.find(x => x.name === n); return { name: n, rawKB: kb(f.raw), gzipKB: kb(f.gzip) }; }),
      css: launch.css,
      rawKB: kb(sum(launchFiles, 'raw')),
      gzipKB: kb(sum(launchFiles, 'gzip')),
      imagesNamedByLaunchJs: launchImages.map(f => ({ name: f.name, rawKB: kb(f.raw) })),
    },
    largestJs: by(/\.js$/).slice(0, 15).map(f => ({ name: f.name, rawKB: kb(f.raw), gzipKB: kb(f.gzip) })),
    images: by(IMAGE).map(f => ({ name: f.name, rawKB: kb(f.raw) })),
  };
}

/** Per-source attribution of the launch chunks (rendered, pre-minify bytes),
 * from one in-memory build. */
async function moduleReport(root) {
  const { build } = await import('vite');
  let bundle;
  await build({
    root,
    logLevel: 'silent',
    build: { write: false, emptyOutDir: false },
    plugins: [{ name: 'perf-report-capture', generateBundle(_o, b) { bundle = b; } }],
  });
  const chunks = Object.values(bundle).filter(c => c.type === 'chunk');
  const byFile = new Map(chunks.map(c => [c.fileName, c]));
  // Same roots as launchSet: the entry and the App chunk main.tsx loads at once.
  const roots = chunks.filter(c => c.isEntry || /^assets\/App-[\w-]+\.js$/.test(c.fileName)).map(c => c.fileName);
  const seen = new Set();
  const stack = [...roots];
  while (stack.length) {
    const f = stack.pop();
    if (seen.has(f) || !byFile.has(f)) continue;
    seen.add(f);
    stack.push(...byFile.get(f).imports);
  }
  const mods = [];
  for (const f of seen) {
    for (const [id, m] of Object.entries(byFile.get(f).modules)) {
      mods.push({ chunk: f.replace(/^assets\//, '').replace(/-[\w-]{8}\.js$/, ''), id: id.replace(root, '').replace(/^\/?/, ''), renderedKB: kb(m.renderedLength) });
    }
  }
  mods.sort((a, b) => b.renderedKB - a.renderedKB);
  const dirs = new Map();
  for (const m of mods) {
    const key = m.id.includes('node_modules/') ? 'node_modules/' + m.id.split('node_modules/')[1].split('/').slice(0, m.id.includes('node_modules/@') ? 2 : 1).join('/') : m.id;
    dirs.set(key, (dirs.get(key) ?? 0) + m.renderedKB);
  }
  return {
    launchChunks: [...seen],
    topModules: mods.slice(0, 40),
    packages: [...dirs.entries()].filter(([k]) => k.startsWith('node_modules/')).sort((a, b) => b[1] - a[1]).slice(0, 15).map(([k, v]) => ({ package: k, renderedKB: +v.toFixed(1) })),
  };
}

async function main(argv) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const dist = join(root, 'dist');
  if (!existsSync(join(dist, 'index.html'))) {
    console.error('perf-report: dist/ not found. Run npm run build first.');
    process.exit(1);
  }
  let commit = null;
  try { commit = execFileSync('git', ['rev-parse', '--short=8', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(); } catch { /* not a checkout */ }
  const report = { commit, builtAt: statSync(join(dist, 'index.html')).mtime.toISOString(), bundle: bundleReport(dist) };
  if (argv.includes('--modules')) report.modules = await moduleReport(root);
  console.log(JSON.stringify(report, null, 2));
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  await main(process.argv.slice(2));
}
