// How the Preview pane shows a text file (run 15 P1): CSV/TSV as a table,
// JSON pretty-printed, code highlighted. Pure, so it is unit-tested; the
// backend's UTF-8 check (chat::preview_file) still decides text vs binary.

/** Highlighting a very large file would build a span per token; above this
 *  many characters the pane shows plain text (still crisp, just uncolored). */
export const HIGHLIGHT_MAX = 200_000;

export type TextView = { kind: 'table' } | { kind: 'json' } | { kind: 'code'; lang?: string };

const LANGS: Record<string, string> = {
  ts: 'ts', tsx: 'tsx', js: 'js', jsx: 'jsx', mjs: 'mjs', cjs: 'cjs', json5: 'json5',
  rs: 'rust', py: 'python', sh: 'sh', bash: 'bash', zsh: 'zsh',
  css: 'css', scss: 'scss', less: 'less', xml: 'xml', vue: 'vue',
  diff: 'diff', patch: 'diff', jsonc: 'jsonc',
};

const extOf = (path: string) => (/\.([a-z0-9]+)$/i.exec(path.split(/[?#]/)[0])?.[1] ?? '').toLowerCase();

/** The view for a text file the pane shows as text (not Markdown, HTML, SVG
 *  or a diagram, which have their own renderers). */
export function textViewOf(path: string): TextView {
  const ext = extOf(path);
  if (ext === 'csv' || ext === 'tsv') return { kind: 'table' };
  if (ext === 'json' || ext === 'geojson' || ext === 'webmanifest') return { kind: 'json' };
  return { kind: 'code', lang: LANGS[ext] };
}

/** `text` pretty-printed when it is JSON, else null (shown as written). */
export function prettyJson(text: string): string | null {
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return null; }
}

/** Whether `text` in `lang` is worth highlighting. */
export const highlights = (text: string, lang: string | undefined): lang is string => !!lang && text.length <= HIGHLIGHT_MAX;
