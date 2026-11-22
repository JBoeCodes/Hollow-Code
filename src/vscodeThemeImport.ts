/**
 * Import a VS Code colour theme as a Hollow theme (t3code parity 4.7, A15).
 * A VS Code theme is JSON with comments (`colors` keyed by UI role, `type`).
 * Hollow's custom themes are the Kitty-style text `themes.rs` reads, so the
 * import writes that text for the person to review and save in the theme
 * editor: nothing is saved or applied here. Only the colours a Hollow theme
 * has slots for are read; syntax token rules and fonts are left out.
 */

export interface VsCodeImport { name: string; source: string; mapped: number; notes: string[] }

/** JSON with `//` and block comments and trailing commas (what theme files use), as a value. */
export function parseJsonc(text: string): unknown {
  let out = '';
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (j < n && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j + 1;
    } else if (c === '/' && text[i + 1] === '/') {
      while (i < n && text[i] !== '\n') i++;
    } else if (c === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      i = end < 0 ? n : end + 2;
    } else {
      out += c;
      i++;
    }
  }
  return JSON.parse(out.replace(/^﻿/, '').replace(/,(\s*[}\]])/g, '$1'));
}

type Rgb = [number, number, number];

/** `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa` laid over `under`, as `#rrggbb`; null when it is not a hex colour. */
export function opaqueHex(value: unknown, under: Rgb = [0, 0, 0]): string | null {
  if (typeof value !== 'string') return null;
  const m = /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(value.trim());
  if (!m) return null;
  let h = m[1];
  if (h.length <= 4) h = [...h].map(ch => ch + ch).join('');
  const rgb: Rgb = [parseInt(h.slice(0, 2), 16), parseInt(h.slice(2, 4), 16), parseInt(h.slice(4, 6), 16)];
  const a = h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1;
  const mixed = rgb.map((v, k) => Math.round(v * a + under[k] * (1 - a)));
  return `#${mixed.map(v => v.toString(16).padStart(2, '0')).join('')}`;
}

const rgbOf = (hex: string): Rgb => [parseInt(hex.slice(1, 3), 16), parseInt(hex.slice(3, 5), 16), parseInt(hex.slice(5, 7), 16)];
const luma = (hex: string) => { const [r, g, b] = rgbOf(hex); return (0.299 * r + 0.587 * g + 0.114 * b) / 255; };

const ANSI = ['Black', 'Red', 'Green', 'Yellow', 'Blue', 'Magenta', 'Cyan', 'White'];
const ACCENT_KEYS = ['focusBorder', 'button.background', 'textLink.foreground', 'activityBarBadge.background', 'progressBar.background', 'editorCursor.foreground'];

/** Convert a VS Code theme file's text to Hollow theme source. Throws a plain-words Error when it cannot. */
export function vscodeThemeToSource(text: string, fileName = ''): VsCodeImport {
  let json: any;
  try { json = parseJsonc(text); } catch { throw new Error('That file is not a VS Code theme: it is not valid JSON.'); }
  if (!json || typeof json !== 'object' || typeof json.colors !== 'object' || !json.colors) {
    throw new Error('That file has no "colors" section, so it is not a VS Code colour theme.');
  }
  const colors = json.colors as Record<string, unknown>;
  const notes: string[] = [];
  if (json.include) notes.push('It includes another theme file that is not read here; colours it only inherits fall back to Hollow’s own.');
  const background = opaqueHex(colors['editor.background']) ?? opaqueHex(colors['sideBar.background']);
  const foreground = opaqueHex(colors['editor.foreground'], rgbOf(background ?? '#000000')) ?? opaqueHex(colors['foreground'], rgbOf(background ?? '#000000'));
  if (!background || !foreground) throw new Error('That theme sets no editor background and foreground colour, which Hollow needs.');
  const bg = rgbOf(background);
  const pick = (keys: string[]) => keys.map(k => opaqueHex(colors[k], bg)).find(Boolean) ?? null;
  const ansi = (k: number) => opaqueHex(colors[`terminal.ansi${k < 8 ? '' : 'Bright'}${ANSI[k % 8]}`], bg);
  const slots = Array.from({ length: 16 }, (_, k) => ansi(k));
  const mapped = slots.filter(Boolean).length;
  const accent = pick(ACCENT_KEYS) ?? slots[4] ?? foreground;
  if (!pick(ACCENT_KEYS)) notes.push('No accent colour was set; Hollow used the terminal blue (or the text colour).');
  const kind = typeof json.type === 'string' ? json.type.toLowerCase() : '';
  const isDark = kind === 'dark' || kind === 'hc' || kind === 'hc-black' ? true : kind === 'light' || kind === 'hc-light' ? false : luma(background) < 0.5;
  const rawName = typeof json.name === 'string' && json.name.trim() ? json.name.trim() : fileName.replace(/\.[^.]+$/, '').trim();
  const name = (rawName || 'Imported theme').replace(/["\\\r\n]/g, '').slice(0, 60);
  const selection = pick(['editor.selectionBackground', 'list.activeSelectionBackground']);
  const cursor = pick(['editorCursor.foreground']);
  const success = pick(['testing.iconPassed', 'gitDecoration.addedResourceForeground']);
  const warning = pick(['editorWarning.foreground', 'list.warningForeground']);
  const error = pick(['editorError.foreground', 'list.errorForeground']);
  if (mapped < 16) notes.push(mapped === 0 ? 'No terminal colours were set; Hollow derives the other colours from these.' : `${mapped} of the 16 terminal colours were set; Hollow derives the rest.`);

  const lines = [
    '# Imported from a VS Code theme. Review it, then Save.',
    `name = "${name}"`,
    `is_dark = ${isDark}`,
    '',
    `accent     = "${accent}"`,
    `foreground = "${foreground}"`,
    `background = "${background}"`,
  ];
  if (cursor) lines.push(`cursor     = "${cursor}"`);
  if (selection) lines.push(`selection_background = "${selection}"`);
  const sf = pick(['editor.selectionForeground']);
  if (sf) lines.push(`selection_foreground = "${sf}"`);
  slots.forEach((c, k) => { if (c) lines.push(`color${k}${k < 10 ? ' ' : ''} = "${c}"`); });
  if (success) lines.push(`success = "${success}"`);
  if (warning) lines.push(`warning = "${warning}"`);
  if (error) lines.push(`error   = "${error}"`);
  return { name, source: lines.join('\n') + '\n', mapped, notes };
}
