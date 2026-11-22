// Key presses to the bytes a shell expects (t3code parity 7.2). Pure: the
// panel hands it the facts of a keydown and sends what comes back. A chord
// with Cmd is never taken (Copy, Paste, Close Window and the rest stay the
// app's and the system's), so the terminal binds no Cmd chord of its own, and
// Shift+Tab is left alone so focus can always get out.

export interface KeyFacts { key: string; code?: string; ctrlKey: boolean; altKey: boolean; metaKey: boolean; shiftKey: boolean }

const CSI_KEYS: Record<string, string> = { ArrowUp: 'A', ArrowDown: 'B', ArrowRight: 'C', ArrowLeft: 'D', Home: 'H', End: 'F' };
const TILDE_KEYS: Record<string, number> = { Insert: 2, Delete: 3, PageUp: 5, PageDown: 6 };
const F_KEYS: Record<string, string> = { F1: 'P', F2: 'Q', F3: 'R', F4: 'S' };
const F_TILDE: Record<string, number> = { F5: 15, F6: 17, F7: 18, F8: 19, F9: 20, F10: 21, F11: 23, F12: 24 };

/** The xterm modifier number: 1 plus shift 1, alt 2, ctrl 4; 1 means none. */
const modifier = (e: KeyFacts) => 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0);

/** The bytes for a key press, or null when the terminal leaves it alone
 *  (a Cmd chord, a bare modifier, a dead key or an IME step). */
export function keyToBytes(e: KeyFacts, opts: { appCursor: boolean }): string | null {
  if (e.metaKey) return null;
  const { key } = e;
  if (key === 'Dead' || key === 'Process' || key === 'Unidentified' || key === 'Shift' || key === 'Control' || key === 'Alt' || key === 'Meta' || key === 'CapsLock') return null;
  const mod = modifier(e);

  if (key === 'Enter') return e.altKey ? '\x1b\r' : '\r';
  // Shift+Tab is never taken: it is how keyboard focus leaves the terminal (a
  // shell's reverse completion is the price; Tab itself goes to the shell).
  if (key === 'Tab') return e.shiftKey ? null : '\t';
  if (key === 'Escape') return '\x1b';
  if (key === 'Backspace') return e.altKey ? '\x1b\x7f' : e.ctrlKey ? '\x08' : '\x7f';
  const csi = CSI_KEYS[key];
  if (csi) {
    // Option+Left and Option+Right move by word in a shell, as Terminal sends them.
    if (e.altKey && !e.ctrlKey && !e.shiftKey && key === 'ArrowLeft') return '\x1bb';
    if (e.altKey && !e.ctrlKey && !e.shiftKey && key === 'ArrowRight') return '\x1bf';
    if (mod > 1) return `\x1b[1;${mod}${csi}`;
    return opts.appCursor ? `\x1bO${csi}` : `\x1b[${csi}`;
  }
  const tilde = TILDE_KEYS[key];
  if (tilde) return mod > 1 ? `\x1b[${tilde};${mod}~` : `\x1b[${tilde}~`;
  const f = F_KEYS[key];
  if (f) return mod > 1 ? `\x1b[1;${mod}${f}` : `\x1bO${f}`;
  const ft = F_TILDE[key];
  if (ft) return mod > 1 ? `\x1b[${ft};${mod}~` : `\x1b[${ft}~`;

  // Ctrl with a letter or one of the punctuation keys that have a control code.
  if (e.ctrlKey && !e.altKey) {
    if (key.length === 1) {
      const c = key.toLowerCase();
      if (c >= 'a' && c <= 'z') return String.fromCharCode(c.charCodeAt(0) - 96);
      if (key === ' ' || key === '@' || key === '2') return '\x00';
      if (key === '[' || key === '3') return '\x1b';
      if (key === '\\' || key === '4') return '\x1c';
      if (key === ']' || key === '5') return '\x1d';
      if (key === '^' || key === '6') return '\x1e';
      if (key === '_' || key === '/' || key === '7') return '\x1f';
      if (key === '?' || key === '8') return '\x7f';
    }
    return null;
  }

  // Option as Meta: the physical key with an ESC prefix (the composed
  // character macOS would type is not what a shell binding wants).
  if (e.altKey) {
    const m = /^Key([A-Z])$/.exec(e.code ?? '');
    if (m) return `\x1b${e.shiftKey ? m[1] : m[1].toLowerCase()}`;
    const d = /^Digit([0-9])$/.exec(e.code ?? '');
    if (d) return `\x1b${d[1]}`;
    if (key.length > 0 && [...key].length === 1) return `\x1b${key}`;
    return null;
  }

  // A printable character (one code point; a named key like "ArrowUp" is longer).
  if ([...key].length === 1) return key;
  return null;
}

/** Text pasted into the terminal: line endings become the Enter a shell
 *  expects, and bracketed paste wraps it when the program asked for that.
 *  Control characters other than tab and newline are dropped, so a pasted
 *  escape sequence cannot end the bracket early or drive the terminal. */
export function pasteBytes(text: string, bracketed: boolean): string {
  const clean = text.replace(/\r\n?|\n/g, '\r').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001a\u001c-\u001f\u007f]/g, '').replace(/\u001b/g, '');
  return bracketed ? `\x1b[200~${clean}\x1b[201~` : clean;
}
