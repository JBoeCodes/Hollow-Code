// The terminal drawer's screen (t3code parity 7.2): a small terminal
// emulator that turns a shell's output into rows of styled cells, with no
// dependency and no vendored code. It covers what shells and the usual
// full-screen tools send: printing with wrap, cursor movement, erase, insert
// and delete, scroll regions, the alternate screen, SGR colour (16, 256 and
// true colour), the line-drawing character set, window titles and the
// answers a program asks for (cursor position, device attributes). It
// ignores what could reach outside the drawer: OSC 52 clipboard writes, OSC 8
// links, mouse reporting and every other sequence it does not know are read
// to their end and dropped. Pure (no DOM, no store), so vitest drives it.

/** Style attribute bits. */
export const ATTR = { bold: 1, dim: 2, italic: 4, underline: 8, inverse: 16, strike: 32, hidden: 64 } as const;
/** A colour: -1 is the theme's default, 0 to 255 the xterm palette, and
 *  `RGB | 0xRRGGBB` a true colour. */
export const DEFAULT_COLOR = -1;
const RGB = 0x1000000;

export interface Cell { ch: string; fg: number; bg: number; attr: number }
const blank = (bg = DEFAULT_COLOR): Cell => ({ ch: ' ', fg: DEFAULT_COLOR, bg, attr: 0 });

export interface TermScreenOptions {
  /** Rows kept after they scroll off the top. */
  scrollback?: number;
  /** Bytes a program asked for, to send back to the shell. */
  onReply?: (data: string) => void;
}

const CHUNK = 50;
/** Width of one character in cells: 0 for a mark that joins the previous one, 2 for wide. */
export function charWidth(cp: number): 0 | 1 | 2 {
  if (cp < 0x300) return 1;
  if ((cp >= 0x300 && cp <= 0x36f) || (cp >= 0x200b && cp <= 0x200f) || (cp >= 0xfe00 && cp <= 0xfe0f) || cp === 0x20e3 || (cp >= 0x1ab0 && cp <= 0x1aff) || (cp >= 0x1dc0 && cp <= 0x1dff) || (cp >= 0x20d0 && cp <= 0x20ff) || cp === 0x2060 || (cp >= 0xe0100 && cp <= 0xe01ef)) return 0;
  if ((cp >= 0x1100 && cp <= 0x115f) || (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) || (cp >= 0xac00 && cp <= 0xd7a3) || (cp >= 0xf900 && cp <= 0xfaff) || (cp >= 0xfe30 && cp <= 0xfe6f) || (cp >= 0xff00 && cp <= 0xff60) || (cp >= 0xffe0 && cp <= 0xffe6) || (cp >= 0x1f300 && cp <= 0x1f64f) || (cp >= 0x1f900 && cp <= 0x1f9ff) || (cp >= 0x1f680 && cp <= 0x1f6ff) || (cp >= 0x20000 && cp <= 0x3fffd)) return 2;
  return 1;
}

/** DEC special graphics (ESC ( 0): the box-drawing set tmux, mc and ncurses use. */
const LINE_DRAWING: Record<string, string> = {
  '`': '◆', a: '▒', f: '°', g: '±', j: '┘', k: '┐', l: '┌', m: '└', n: '┼', o: '⎺', p: '⎻', q: '─', r: '⎼', s: '⎽', t: '├', u: '┤', v: '┴', w: '┬', x: '│', y: '≤', z: '≥', '{': 'π', '|': '≠', '}': '£', '~': '·',
};

const ANSI16 = ['#000000', '#cd3131', '#0dbc79', '#e5e510', '#2472c8', '#bc3fbc', '#11a8cd', '#e5e5e5', '#666666', '#f14c4c', '#23d18b', '#f5f543', '#3b8eea', '#d670d6', '#29b8db', '#ffffff'];
const hex2 = (n: number) => n.toString(16).padStart(2, '0');
/** `#rrggbb` for a palette index from 16 up (the 6x6x6 cube, then greys). */
export function paletteHex(n: number): string {
  if (n < 16) return ANSI16[n] ?? '#ffffff';
  if (n < 232) {
    const i = n - 16;
    const level = (v: number) => (v === 0 ? 0 : 55 + v * 40);
    return `#${hex2(level(Math.floor(i / 36)))}${hex2(level(Math.floor(i / 6) % 6))}${hex2(level(i % 6))}`;
  }
  const g = 8 + (n - 232) * 10;
  return `#${hex2(g)}${hex2(g)}${hex2(g)}`;
}

const esc = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** The cell's colour as CSS, null for the theme default. A true colour or a
 *  palette index above 15 is an inline value built only from numbers. */
function cssColor(c: number): { cls?: number; hex?: string } | null {
  if (c === DEFAULT_COLOR) return null;
  if (c >= RGB) return { hex: `#${hex2((c >> 16) & 255)}${hex2((c >> 8) & 255)}${hex2(c & 255)}` };
  if (c < 16) return { cls: c };
  return { hex: paletteHex(c) };
}

/** One row as HTML: runs of equal style, text escaped, trailing default blanks cut. */
export function rowToHtml(row: readonly Cell[]): string {
  let end = row.length;
  while (end > 0 && row[end - 1].ch === ' ' && row[end - 1].bg === DEFAULT_COLOR && !(row[end - 1].attr & ATTR.inverse)) end--;
  let html = '';
  let i = 0;
  while (i < end) {
    const first = row[i];
    let text = '';
    let j = i;
    while (j < end && row[j].fg === first.fg && row[j].bg === first.bg && row[j].attr === first.attr) { text += row[j].ch; j++; }
    i = j;
    const inv = (first.attr & ATTR.inverse) !== 0;
    const fgSrc = inv ? first.bg : first.fg;
    const bgSrc = inv ? first.fg : first.bg;
    const fg = cssColor(fgSrc);
    const bg = cssColor(bgSrc);
    const classes: string[] = [];
    if (first.attr & ATTR.bold) classes.push('tb');
    if (first.attr & ATTR.dim) classes.push('td');
    if (first.attr & ATTR.italic) classes.push('ti');
    const lines = [first.attr & ATTR.underline ? 'underline' : '', first.attr & ATTR.strike ? 'line-through' : ''].filter(Boolean);
    if (lines.length) classes.push(lines.length === 2 ? 'tus' : lines[0] === 'underline' ? 'tu' : 'ts');
    if (first.attr & ATTR.hidden) classes.push('th');
    if (fg?.cls !== undefined) classes.push(`tf${fg.cls}`);
    else if (inv && !fg) classes.push('tfinv');
    if (bg?.cls !== undefined) classes.push(`tg${bg.cls}`);
    else if (inv && !bg) classes.push('tginv');
    const style = [fg?.hex ? `color:${fg.hex}` : '', bg?.hex ? `background:${bg.hex}` : ''].filter(Boolean).join(';');
    const safe = esc(text);
    html += classes.length || style ? `<span${classes.length ? ` class="${classes.join(' ')}"` : ''}${style ? ` style="${style}"` : ''}>${safe}</span>` : safe;
  }
  return html;
}

type Mode = 'ground' | 'esc' | 'csi' | 'osc' | 'oscEsc' | 'string' | 'stringEsc' | 'charset';

interface Saved { row: number; col: number; fg: number; bg: number; attr: number; wrapPending: boolean; graphics: boolean }

export class TermScreen {
  rows: number;
  cols: number;
  title = '';
  cursorRow = 0;
  cursorCol = 0;
  cursorVisible = true;
  /** DECCKM: arrows send ESC O instead of ESC [. */
  appCursorKeys = false;
  /** Mode 2004: a paste is wrapped so the shell can tell it from typing. */
  bracketedPaste = false;
  altScreen = false;
  /** Bumped by every write and resize; the view re-reads when it changes. */
  version = 0;

  private grid: Cell[][];
  private mainGrid: Cell[][] | null = null;
  private mainCursor: Saved | null = null;
  private cache: (string | null)[];
  private sbChunks: string[] = [];
  private sbTail: string[] = [];
  private sbLimit: number;
  private sbPlain: string[] = [];
  private top = 0;
  private bottom: number;
  private fg = DEFAULT_COLOR;
  private bg = DEFAULT_COLOR;
  private attr = 0;
  private wrapPending = false;
  private autoWrap = true;
  private graphics = false;
  private saved: Saved | null = null;
  private tabs: boolean[];
  private mode: Mode = 'ground';
  private params = '';
  private osc = '';
  private charsetFor = '(';
  private reply: (data: string) => void;
  /** An unfinished UTF-16 pair carried between writes. */
  private high = '';

  constructor(rows: number, cols: number, opts: TermScreenOptions = {}) {
    this.rows = Math.max(1, rows);
    this.cols = Math.max(1, cols);
    this.sbLimit = opts.scrollback ?? 2000;
    this.reply = opts.onReply ?? (() => {});
    this.grid = Array.from({ length: this.rows }, () => this.blankRow());
    this.cache = new Array(this.rows).fill(null);
    this.bottom = this.rows - 1;
    this.tabs = Array.from({ length: this.cols }, (_, i) => i % 8 === 0 && i > 0);
  }

  private blankRow(): Cell[] { return Array.from({ length: this.cols }, () => blank(this.bg)); }

  // ---- reading the screen ----

  /** The screen's rows as HTML, each cached until it changes. */
  rowsHtml(): string[] {
    return this.grid.map((row, i) => this.cache[i] ?? (this.cache[i] = rowToHtml(row)));
  }
  /** Scrollback as full blocks of rows (each one string, unchanging once full). */
  scrollbackChunks(): readonly string[] { return this.sbChunks; }
  /** The rows of scrollback after the last full block. */
  scrollbackTail(): string { return this.sbTail.join(''); }
  scrollbackCount(): number { return this.sbChunks.length * CHUNK + this.sbTail.length; }
  /** The screen's rows as plain text, trailing blanks cut. */
  screenText(): string[] { return this.grid.map(row => row.map(c => c.ch).join('').replace(/\s+$/, '')); }
  /** Scrollback and screen as plain text (Copy all). */
  allText(): string {
    const lines = [...this.sbPlain, ...this.screenText()];
    while (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines.join('\n');
  }

  // ---- writing ----

  write(data: string): void {
    let text = this.high + data;
    this.high = '';
    const last = text.charCodeAt(text.length - 1);
    if (last >= 0xd800 && last <= 0xdbff) { this.high = text.slice(-1); text = text.slice(0, -1); }
    for (const ch of text) this.feed(ch);
    this.version++;
  }

  private feed(ch: string): void {
    const code = ch.codePointAt(0)!;
    switch (this.mode) {
      case 'ground': return this.ground(ch, code);
      case 'esc': return this.afterEsc(ch);
      case 'charset': {
        if (this.charsetFor === '(') this.graphics = ch === '0';
        this.mode = 'ground';
        return;
      }
      case 'csi': return this.csiByte(ch, code);
      case 'osc':
        if (code === 7) { this.finishOsc(); return; }
        if (code === 0x1b) { this.mode = 'oscEsc'; return; }
        if (this.osc.length < 2048) this.osc += ch;
        return;
      case 'oscEsc':
        if (ch === '\\') this.finishOsc();
        else { this.mode = 'ground'; this.osc = ''; this.afterEsc(ch); }
        return;
      case 'string':
        if (code === 7) this.mode = 'ground';
        else if (code === 0x1b) this.mode = 'stringEsc';
        return;
      case 'stringEsc':
        this.mode = ch === '\\' ? 'ground' : 'string';
        return;
    }
  }

  private ground(ch: string, code: number): void {
    if (code < 0x20) return this.control(code);
    if (code === 0x7f) return;
    if (code >= 0x80 && code < 0xa0) return;
    this.print(this.graphics && LINE_DRAWING[ch] ? LINE_DRAWING[ch] : ch, code);
  }

  private control(code: number): void {
    switch (code) {
      case 0x1b: this.mode = 'esc'; return;
      case 0x08: this.wrapPending = false; if (this.cursorCol > 0) this.cursorCol--; return;
      case 0x09: this.wrapPending = false; this.cursorCol = this.nextTab(this.cursorCol); return;
      case 0x0a: case 0x0b: case 0x0c: this.index(); return;
      case 0x0d: this.wrapPending = false; this.cursorCol = 0; return;
      default: return;
    }
  }

  private nextTab(col: number): number {
    for (let c = col + 1; c < this.cols; c++) if (this.tabs[c]) return c;
    return this.cols - 1;
  }

  private touch(row: number): void { this.cache[row] = null; }

  private print(ch: string, code: number): void {
    const width = charWidth(code);
    if (width === 0) {
      // A joining mark rides on the cell before the cursor.
      const col = this.wrapPending ? this.cursorCol : this.cursorCol - 1;
      if (col >= 0) { this.grid[this.cursorRow][col].ch += ch; this.touch(this.cursorRow); }
      return;
    }
    if (this.wrapPending && this.autoWrap) { this.cursorCol = 0; this.index(); }
    this.wrapPending = false;
    if (width === 2 && this.cursorCol === this.cols - 1) {
      if (this.autoWrap) { this.cursorCol = 0; this.index(); } else return;
    }
    const row = this.grid[this.cursorRow];
    if (width === 2 && this.cursorCol + 1 < this.cols) row[this.cursorCol + 1] = { ch: '', fg: this.fg, bg: this.bg, attr: this.attr };
    row[this.cursorCol] = { ch, fg: this.fg, bg: this.bg, attr: this.attr };
    this.touch(this.cursorRow);
    const next = this.cursorCol + width;
    if (next >= this.cols) { this.cursorCol = this.cols - 1; this.wrapPending = true; } else this.cursorCol = next;
  }

  /** Down one row, scrolling at the bottom of the region. */
  private index(): void {
    this.wrapPending = false;
    if (this.cursorRow === this.bottom) this.scrollUp(1);
    else if (this.cursorRow < this.rows - 1) this.cursorRow++;
  }

  private reverseIndex(): void {
    this.wrapPending = false;
    if (this.cursorRow === this.top) this.scrollDown(1);
    else if (this.cursorRow > 0) this.cursorRow--;
  }

  private pushScrollback(row: Cell[]): void {
    this.sbTail.push(`<div class="trow">${rowToHtml(row)}</div>`);
    this.sbPlain.push(row.map(c => c.ch).join('').replace(/\s+$/, ''));
    if (this.sbTail.length >= CHUNK) { this.sbChunks.push(this.sbTail.join('')); this.sbTail = []; }
    while (this.sbChunks.length * CHUNK > this.sbLimit && this.sbChunks.length > 0) this.sbChunks.shift();
    if (this.sbPlain.length > this.sbLimit + CHUNK) this.sbPlain.splice(0, this.sbPlain.length - this.sbLimit);
  }

  private scrollUp(n: number): void {
    for (let k = 0; k < n; k++) {
      const [gone] = this.grid.splice(this.top, 1);
      if (!this.altScreen && this.top === 0) this.pushScrollback(gone);
      this.grid.splice(this.bottom, 0, this.blankRow());
    }
    for (let i = this.top; i <= this.bottom; i++) this.touch(i);
  }

  private scrollDown(n: number): void {
    for (let k = 0; k < n; k++) {
      this.grid.splice(this.bottom, 1);
      this.grid.splice(this.top, 0, this.blankRow());
    }
    for (let i = this.top; i <= this.bottom; i++) this.touch(i);
  }

  private afterEsc(ch: string): void {
    this.mode = 'ground';
    switch (ch) {
      case '[': this.mode = 'csi'; this.params = ''; return;
      case ']': this.mode = 'osc'; this.osc = ''; return;
      case 'P': case '_': case '^': case 'X': this.mode = 'string'; return;
      case '(': case ')': case '*': case '+': this.mode = 'charset'; this.charsetFor = ch; return;
      case '7': this.saved = this.snapshot(); return;
      case '8': if (this.saved) this.restore(this.saved); return;
      case 'D': this.index(); return;
      case 'E': this.cursorCol = 0; this.index(); return;
      case 'M': this.reverseIndex(); return;
      case 'H': this.tabs[this.cursorCol] = true; return;
      case 'c': this.fullReset(); return;
      default: return;
    }
  }

  private snapshot(): Saved {
    return { row: this.cursorRow, col: this.cursorCol, fg: this.fg, bg: this.bg, attr: this.attr, wrapPending: this.wrapPending, graphics: this.graphics };
  }
  private restore(s: Saved): void {
    this.cursorRow = Math.min(s.row, this.rows - 1);
    this.cursorCol = Math.min(s.col, this.cols - 1);
    this.fg = s.fg; this.bg = s.bg; this.attr = s.attr; this.wrapPending = s.wrapPending; this.graphics = s.graphics;
  }

  private fullReset(): void {
    this.fg = DEFAULT_COLOR; this.bg = DEFAULT_COLOR; this.attr = 0;
    this.top = 0; this.bottom = this.rows - 1;
    this.cursorRow = 0; this.cursorCol = 0; this.wrapPending = false; this.autoWrap = true; this.graphics = false;
    this.cursorVisible = true; this.appCursorKeys = false; this.bracketedPaste = false;
    if (this.altScreen) this.leaveAlt(false);
    this.grid = Array.from({ length: this.rows }, () => this.blankRow());
    this.cache = new Array(this.rows).fill(null);
  }

  private finishOsc(): void {
    this.mode = 'ground';
    const sep = this.osc.indexOf(';');
    const kind = sep < 0 ? this.osc : this.osc.slice(0, sep);
    // Only the title is honoured: no clipboard (52), links (8) or file hints (7).
    if (kind === '0' || kind === '2') this.title = this.osc.slice(sep + 1).replace(/[\u0000-\u001f\u007f]/g, '').slice(0, 200);
    this.osc = '';
  }

  // ---- CSI ----

  private csiByte(ch: string, code: number): void {
    if (code >= 0x30 && code <= 0x3f) { if (this.params.length < 256) this.params += ch; return; }
    if (code >= 0x20 && code <= 0x2f) { if (this.params.length < 256) this.params += ch; return; }
    this.mode = 'ground';
    if (code < 0x40 || code > 0x7e) return;
    this.csi(ch, this.params);
  }

  private nums(raw: string, fallback: number): number[] {
    const body = raw.replace(/^[?>=!]/, '').replace(/[\x20-\x2f]+$/, '');
    if (body === '') return [fallback];
    return body.split(/[;:]/).map(p => (p === '' ? fallback : Math.min(parseInt(p, 10) || 0, 65535)));
  }

  private csi(final: string, raw: string): void {
    const priv = raw.startsWith('?');
    const p = this.nums(raw, 0);
    const one = (i = 0) => Math.max(1, p[i] || 0);
    switch (final) {
      case 'A': this.moveTo(this.cursorRow - one(), this.cursorCol); return;
      case 'B': case 'e': this.moveTo(this.cursorRow + one(), this.cursorCol); return;
      case 'C': case 'a': this.moveTo(this.cursorRow, this.cursorCol + one()); return;
      case 'D': this.moveTo(this.cursorRow, this.cursorCol - one()); return;
      case 'E': this.moveTo(this.cursorRow + one(), 0); return;
      case 'F': this.moveTo(this.cursorRow - one(), 0); return;
      case 'G': case '`': this.moveTo(this.cursorRow, one() - 1); return;
      case 'd': this.moveTo(one() - 1, this.cursorCol); return;
      case 'H': case 'f': this.moveTo(one(0) - 1, one(1) - 1); return;
      case 'I': for (let k = 0; k < one(); k++) this.cursorCol = this.nextTab(this.cursorCol); this.wrapPending = false; return;
      case 'J': this.eraseDisplay(p[0] || 0); return;
      case 'K': this.eraseLine(p[0] || 0); return;
      case 'L': this.insertLines(one()); return;
      case 'M': this.deleteLines(one()); return;
      case 'P': this.deleteChars(one()); return;
      case '@': this.insertChars(one()); return;
      case 'X': this.eraseChars(one()); return;
      case 'S': this.scrollUp(one()); return;
      case 'T': this.scrollDown(one()); return;
      case 'm': this.sgr(raw); return;
      case 'r': {
        const t = one(0) - 1;
        const b = (p[1] || this.rows) - 1;
        if (t < b && b < this.rows) { this.top = t; this.bottom = b; } else { this.top = 0; this.bottom = this.rows - 1; }
        this.cursorRow = 0; this.cursorCol = 0; this.wrapPending = false;
        return;
      }
      case 's': if (!priv) this.saved = this.snapshot(); return;
      case 'u': if (this.saved) this.restore(this.saved); return;
      case 'g': if (p[0] === 3) this.tabs.fill(false); else if (p[0] === 0) this.tabs[this.cursorCol] = false; return;
      case 'h': this.setMode(raw, true); return;
      case 'l': this.setMode(raw, false); return;
      case 'n':
        if (!priv && p[0] === 5) this.reply('\x1b[0n');
        else if (!priv && p[0] === 6) this.reply(`\x1b[${this.cursorRow + 1};${this.cursorCol + 1}R`);
        return;
      case 'c': if (!raw.startsWith('>') && (p[0] || 0) === 0) this.reply('\x1b[?1;2c'); return;
      default: return;
    }
  }

  private moveTo(row: number, col: number): void {
    this.wrapPending = false;
    this.cursorRow = Math.min(Math.max(row, 0), this.rows - 1);
    this.cursorCol = Math.min(Math.max(col, 0), this.cols - 1);
  }

  private setMode(raw: string, on: boolean): void {
    if (!raw.startsWith('?')) return;
    for (const n of this.nums(raw, 0)) {
      switch (n) {
        case 1: this.appCursorKeys = on; break;
        case 7: this.autoWrap = on; break;
        case 25: this.cursorVisible = on; break;
        case 47: case 1047: if (on) this.enterAlt(false); else this.leaveAlt(false); break;
        case 1049: if (on) this.enterAlt(true); else this.leaveAlt(true); break;
        case 2004: this.bracketedPaste = on; break;
        default: break;
      }
    }
  }

  private enterAlt(saveCursor: boolean): void {
    if (this.altScreen) return;
    this.mainGrid = this.grid;
    this.mainCursor = saveCursor ? this.snapshot() : null;
    this.altScreen = true;
    this.grid = Array.from({ length: this.rows }, () => this.blankRow());
    this.cache = new Array(this.rows).fill(null);
    this.top = 0; this.bottom = this.rows - 1;
  }

  private leaveAlt(restoreCursor: boolean): void {
    if (!this.altScreen) return;
    this.altScreen = false;
    if (this.mainGrid) this.grid = this.mainGrid;
    this.mainGrid = null;
    this.cache = new Array(this.rows).fill(null);
    this.top = 0; this.bottom = this.rows - 1;
    if (restoreCursor && this.mainCursor) this.restore(this.mainCursor);
    this.mainCursor = null;
  }

  private eraseCells(row: number, from: number, to: number): void {
    const cells = this.grid[row];
    for (let c = Math.max(0, from); c < Math.min(this.cols, to); c++) cells[c] = blank(this.bg);
    this.touch(row);
  }

  private eraseLine(kind: number): void {
    if (kind === 0) this.eraseCells(this.cursorRow, this.cursorCol, this.cols);
    else if (kind === 1) this.eraseCells(this.cursorRow, 0, this.cursorCol + 1);
    else this.eraseCells(this.cursorRow, 0, this.cols);
  }

  private eraseDisplay(kind: number): void {
    if (kind === 0) {
      this.eraseCells(this.cursorRow, this.cursorCol, this.cols);
      for (let r = this.cursorRow + 1; r < this.rows; r++) this.eraseCells(r, 0, this.cols);
    } else if (kind === 1) {
      for (let r = 0; r < this.cursorRow; r++) this.eraseCells(r, 0, this.cols);
      this.eraseCells(this.cursorRow, 0, this.cursorCol + 1);
    } else if (kind === 2) {
      for (let r = 0; r < this.rows; r++) this.eraseCells(r, 0, this.cols);
    } else if (kind === 3) {
      this.sbChunks = []; this.sbTail = []; this.sbPlain = [];
    }
  }

  private insertLines(n: number): void {
    if (this.cursorRow < this.top || this.cursorRow > this.bottom) return;
    for (let k = 0; k < Math.min(n, this.bottom - this.cursorRow + 1); k++) {
      this.grid.splice(this.bottom, 1);
      this.grid.splice(this.cursorRow, 0, this.blankRow());
    }
    for (let i = this.cursorRow; i <= this.bottom; i++) this.touch(i);
    this.cursorCol = 0; this.wrapPending = false;
  }

  private deleteLines(n: number): void {
    if (this.cursorRow < this.top || this.cursorRow > this.bottom) return;
    for (let k = 0; k < Math.min(n, this.bottom - this.cursorRow + 1); k++) {
      this.grid.splice(this.cursorRow, 1);
      this.grid.splice(this.bottom, 0, this.blankRow());
    }
    for (let i = this.cursorRow; i <= this.bottom; i++) this.touch(i);
    this.cursorCol = 0; this.wrapPending = false;
  }

  private deleteChars(n: number): void {
    const row = this.grid[this.cursorRow];
    row.splice(this.cursorCol, Math.min(n, this.cols - this.cursorCol));
    while (row.length < this.cols) row.push(blank(this.bg));
    this.touch(this.cursorRow);
    this.wrapPending = false;
  }

  private insertChars(n: number): void {
    const row = this.grid[this.cursorRow];
    for (let k = 0; k < Math.min(n, this.cols - this.cursorCol); k++) row.splice(this.cursorCol, 0, blank(this.bg));
    row.length = this.cols;
    this.touch(this.cursorRow);
    this.wrapPending = false;
  }

  private eraseChars(n: number): void {
    this.eraseCells(this.cursorRow, this.cursorCol, this.cursorCol + n);
  }

  private sgr(raw: string): void {
    const parts = raw === '' ? ['0'] : raw.split(';');
    for (let i = 0; i < parts.length; i++) {
      const sub = parts[i].split(':');
      const n = parseInt(sub[0] || '0', 10) || 0;
      if (n === 0) { this.fg = DEFAULT_COLOR; this.bg = DEFAULT_COLOR; this.attr = 0; }
      else if (n === 1) this.attr |= ATTR.bold;
      else if (n === 2) this.attr |= ATTR.dim;
      else if (n === 3) this.attr |= ATTR.italic;
      else if (n === 4) this.attr |= ATTR.underline;
      else if (n === 7) this.attr |= ATTR.inverse;
      else if (n === 8) this.attr |= ATTR.hidden;
      else if (n === 9) this.attr |= ATTR.strike;
      else if (n === 22) this.attr &= ~(ATTR.bold | ATTR.dim);
      else if (n === 23) this.attr &= ~ATTR.italic;
      else if (n === 24) this.attr &= ~ATTR.underline;
      else if (n === 27) this.attr &= ~ATTR.inverse;
      else if (n === 28) this.attr &= ~ATTR.hidden;
      else if (n === 29) this.attr &= ~ATTR.strike;
      else if (n >= 30 && n <= 37) this.fg = n - 30;
      else if (n === 39) this.fg = DEFAULT_COLOR;
      else if (n >= 40 && n <= 47) this.bg = n - 40;
      else if (n === 49) this.bg = DEFAULT_COLOR;
      else if (n >= 90 && n <= 97) this.fg = n - 90 + 8;
      else if (n >= 100 && n <= 107) this.bg = n - 100 + 8;
      else if (n === 38 || n === 48) {
        // 38;5;n and 38;2;r;g;b, or the colon forms 38:5:n and 38:2::r:g:b.
        const args = sub.length > 1 ? sub.slice(1).map(v => parseInt(v || '0', 10) || 0) : null;
        let color: number | null = null;
        if (args) {
          if (args[0] === 5) color = args[1] ?? 0;
          else if (args[0] === 2) { const rgb = args.length >= 5 ? args.slice(2, 5) : args.slice(1, 4); color = RGB | ((rgb[0] & 255) << 16) | ((rgb[1] & 255) << 8) | (rgb[2] & 255); }
        } else {
          const mode = parseInt(parts[i + 1] ?? '', 10);
          if (mode === 5) { color = (parseInt(parts[i + 2] ?? '0', 10) || 0) & 255; i += 2; }
          else if (mode === 2) {
            const [r, g, b] = [1, 2, 3].map(k => (parseInt(parts[i + 1 + k] ?? '0', 10) || 0) & 255);
            color = RGB | (r << 16) | (g << 8) | b; i += 4;
          }
        }
        if (color !== null) { if (n === 38) this.fg = color; else this.bg = color; }
      }
    }
  }

  // ---- size ----

  resize(rows: number, cols: number): void {
    rows = Math.max(1, rows); cols = Math.max(1, cols);
    if (rows === this.rows && cols === this.cols) return;
    const fit = (grid: Cell[][]) => {
      for (const row of grid) {
        if (row.length > cols) row.length = cols;
        while (row.length < cols) row.push(blank());
      }
    };
    // A shorter screen pushes the top rows into scrollback, keeping the cursor line in view.
    if (rows < this.rows) {
      const drop = Math.max(0, this.cursorRow - (rows - 1));
      for (let k = 0; k < drop; k++) {
        const gone = this.grid.shift()!;
        if (!this.altScreen) this.pushScrollback(gone);
        this.cursorRow--;
      }
      this.grid.length = Math.min(this.grid.length, rows);
    }
    while (this.grid.length < rows) this.grid.push(Array.from({ length: cols }, () => blank()));
    this.rows = rows;
    this.cols = cols;
    fit(this.grid);
    if (this.mainGrid) {
      while (this.mainGrid.length < rows) this.mainGrid.push(Array.from({ length: cols }, () => blank()));
      this.mainGrid.length = rows;
      fit(this.mainGrid);
    }
    this.top = 0; this.bottom = rows - 1;
    this.cursorRow = Math.min(this.cursorRow, rows - 1);
    this.cursorCol = Math.min(this.cursorCol, cols - 1);
    this.wrapPending = false;
    this.tabs = Array.from({ length: cols }, (_, i) => i % 8 === 0 && i > 0);
    this.cache = new Array(rows).fill(null);
    this.version++;
  }
}
