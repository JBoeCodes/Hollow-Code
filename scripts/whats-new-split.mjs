// What's new, split at build (ST1, plan completion night 2026-10-14).
//
// docs/WHATS_NEW.md stays one file, newest release on top. The What's new
// page was cut overnight 11-08 S4, and vite no longer mounts this splitter.
// The split stays so a later reader can cut the log at a release boundary.
//
// `recent + earlier` is the file byte for byte, cut only at a top-level
// `# ` heading outside a code fence, so joining the two is the whole file.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export const RECENT_BYTES = 40 * 1024;

/** The release title `parseWhatsNew` gives a `# ` heading line. */
export function releaseTitle(line) {
  return line.replace(/^#\s+/, '').trim().replace(/\[([^\]]*)\]\([^)]*\)/g, '$1').trim().replace(/^What['’]s new:\s*/i, '');
}

/** Split What's new into `{ recent, earlier, earlierTitles }`. */
export function splitWhatsNew(md, cap = RECENT_BYTES) {
  const starts = [];
  let fence = false;
  let at = 0;
  for (const line of md.split('\n')) {
    if (/^(```|~~~)/.test(line)) fence = !fence;
    else if (!fence && /^# .+/.test(line)) starts.push({ at, title: releaseTitle(line) });
    at += line.length + 1;
  }
  // Keep whole releases while they fit; the first always stays.
  let cut = md.length;
  for (let i = 1; i < starts.length; i++) {
    const end = starts[i + 1]?.at ?? md.length;
    if (Buffer.byteLength(md.slice(0, end)) > cap) { cut = starts[i].at; break; }
  }
  const earlierTitles = starts.filter(s => s.at >= cut).map(s => s.title);
  return { recent: md.slice(0, cut), earlier: md.slice(cut), earlierTitles };
}

const ID = 'virtual:whats-new-recent';
const DEV_EARLIER_URL = '/__hollow/whats-new-earlier.md';

/** The Vite plugin (vite.config.ts): `virtual:whats-new-recent` is the
 *  recent half plus `EARLIER_TITLES` and the local text asset `EARLIER_URL`.
 *  Types for both are in src/whatsNewModules.d.ts. */
export function whatsNewPlugin(file = fileURLToPath(new URL('../docs/WHATS_NEW.md', import.meta.url))) {
  let build = false;
  return {
    name: 'hollow-whats-new',
    configResolved(config) { build = config.command === 'build'; },
    configureServer(server) {
      server.middlewares.use(DEV_EARLIER_URL, (_req, res, next) => {
        try {
          res.setHeader('Content-Type', 'text/plain; charset=utf-8');
          res.setHeader('Cache-Control', 'no-store');
          res.end(splitWhatsNew(readFileSync(file, 'utf8')).earlier);
        } catch (error) { next(error); }
      });
    },
    resolveId: id => (id === ID ? `\0${id}` : null),
    load(id) {
      if (id !== `\0${ID}`) return null;
      this.addWatchFile(file);
      const { recent, earlier, earlierTitles } = splitWhatsNew(readFileSync(file, 'utf8'));
      const url = build
        ? `import.meta.ROLLUP_FILE_URL_${this.emitFile({ type: 'asset', name: 'whats-new-earlier.md', source: earlier })}`
        : JSON.stringify(DEV_EARLIER_URL);
      return `export default ${JSON.stringify(recent)};\nexport const EARLIER_TITLES = ${JSON.stringify(earlierTitles)};\nexport const EARLIER_URL = ${url};`;
    },
  };
}
