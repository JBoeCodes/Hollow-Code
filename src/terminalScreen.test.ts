import { describe, expect, it } from 'vitest';
import { ATTR, TermScreen, charWidth, paletteHex, rowToHtml } from './terminalScreen';

const screen = (rows = 5, cols = 20, replies?: string[]) => new TermScreen(rows, cols, { onReply: replies ? d => replies.push(d) : undefined });

describe('terminal screen: printing', () => {
  it('prints text, moves on CR and LF, and wraps at the last column', () => {
    const s = screen(4, 5);
    s.write('abcde');
    expect(s.cursorCol).toBe(4); // wrap is pending, not taken
    s.write('f');
    expect(s.screenText().slice(0, 2)).toEqual(['abcde', 'f']);
    s.write('\r\nxy');
    expect(s.screenText()[2]).toBe('xy');
  });

  it('scrolls the top row into scrollback when output runs past the bottom', () => {
    const s = screen(3, 8);
    for (let i = 1; i <= 7; i++) s.write(`line${i}\r\n`);
    expect(s.screenText()).toEqual(['line6', 'line7', '']);
    expect(s.scrollbackCount()).toBe(5);
    expect(s.allText().split('\n')).toEqual(['line1', 'line2', 'line3', 'line4', 'line5', 'line6', 'line7']);
  });

  it('keeps a bounded scrollback', () => {
    const s = new TermScreen(2, 10, { scrollback: 100 });
    for (let i = 0; i < 1000; i++) s.write(`n${i}\r\n`);
    expect(s.scrollbackCount()).toBeLessThanOrEqual(150);
    expect(s.allText().endsWith('n999')).toBe(true);
  });

  it('joins a combining mark to the character before it and gives a wide one two cells', () => {
    const s = screen(2, 6);
    s.write('éx');
    expect(s.screenText()[0]).toBe('éx');
    expect(s.cursorCol).toBe(2);
    s.write('\r\n中z');
    expect(s.cursorCol).toBe(3);
    expect(charWidth(0x4e2d)).toBe(2);
    expect(charWidth(0x41)).toBe(1);
    expect(charWidth(0x301)).toBe(0);
  });

  it('handles tabs and backspace', () => {
    const s = screen(2, 20);
    s.write('a\tb\bc');
    expect(s.screenText()[0]).toBe('a       c');
  });

  it('holds half of a surrogate pair until the other half arrives', () => {
    const s = screen(2, 10);
    const smile = '\u{1f600}';
    s.write(`a${smile[0]}`);
    s.write(`${smile[1]}b`);
    expect(s.screenText()[0]).toBe(`a${smile}b`);
  });
});

describe('terminal screen: cursor and erase', () => {
  it('moves the cursor absolutely and relatively and clamps at the edges', () => {
    const s = screen(5, 10);
    s.write('\x1b[3;4HX');
    expect(s.screenText()[2]).toBe('   X');
    s.write('\x1b[99;99HY');
    expect(s.screenText()[4]).toBe('         Y');
    s.write('\x1b[2A\x1b[3D');
    expect(s.cursorRow).toBe(2);
    expect(s.cursorCol).toBe(6);
    s.write('\x1b[1;1H\x1b[0G');
    expect(s.cursorCol).toBe(0);
  });

  it('erases in line and display', () => {
    const s = screen(3, 6);
    s.write('aaaaaa\r\nbbbbbb\r\ncccccc');
    s.write('\x1b[2;3H\x1b[K');
    expect(s.screenText()[1]).toBe('bb');
    s.write('\x1b[2;2H\x1b[1K');
    expect(s.screenText()[1]).toBe('');
    s.write('\x1b[2;1H\x1b[J');
    expect(s.screenText()).toEqual(['aaaaaa', '', '']);
    s.write('\x1b[2J');
    expect(s.screenText()).toEqual(['', '', '']);
  });

  it('inserts and deletes characters and lines', () => {
    const s = screen(4, 8);
    s.write('abcdef\x1b[1;3H\x1b[2P');
    expect(s.screenText()[0]).toBe('abef');
    s.write('\x1b[1;3H\x1b[2@');
    expect(s.screenText()[0]).toBe('ab  ef');
    s.write('\x1b[1;1H\x1b[X');
    expect(s.screenText()[0]).toBe(' b  ef');
    s.write('\x1b[2;1H1\r\n2\r\n3');
    s.write('\x1b[2;1H\x1b[M');
    expect(s.screenText().slice(1, 4)).toEqual(['2', '3', '']);
    s.write('\x1b[2;1H\x1b[L');
    expect(s.screenText().slice(1, 4)).toEqual(['', '2', '3']);
  });

  it('scrolls inside a region and leaves the rows outside it alone', () => {
    const s = screen(5, 4);
    s.write('\x1b[1;1Hr1\x1b[2;1Hr2\x1b[3;1Hr3\x1b[4;1Hr4\x1b[5;1Hr5');
    s.write('\x1b[2;4r\x1b[4;1H\n');
    expect(s.screenText()).toEqual(['r1', 'r3', 'r4', '', 'r5']);
    s.write('\x1b[2;1H\x1bM');
    expect(s.screenText()).toEqual(['r1', '', 'r3', 'r4', 'r5']);
  });

  it('saves and restores the cursor', () => {
    const s = screen(3, 10);
    s.write('ab\x1b7\x1b[3;5Hz\x1b8c');
    expect(s.screenText()[0]).toBe('abc');
    s.write('\x1b[2;2H\x1b[s\x1b[1;1H\x1b[uq');
    expect(s.screenText()[1]).toBe(' q');
  });
});

describe('terminal screen: colour', () => {
  it('reads 16, 256 and true colour and the attribute bits', () => {
    const s = screen(2, 30);
    s.write('\x1b[1;31mred\x1b[0m \x1b[38;5;202mo\x1b[48;2;1;2;3mT\x1b[0m \x1b[7mi\x1b[0m');
    const html = s.rowsHtml()[0];
    expect(html).toContain('class="tb tf1">red</span>');
    expect(html).toContain('color:#ff5f00');
    expect(html).toContain('background:#010203');
    expect(html).toContain('tfinv tginv');
    expect(rowToHtml([{ ch: 'x', fg: -1, bg: -1, attr: ATTR.underline }])).toBe('<span class="tu">x</span>');
  });

  it('reads the colon forms and the bright ranges', () => {
    const s = screen(2, 20);
    s.write('\x1b[38:2::10:20:30mA\x1b[0m\x1b[91mB\x1b[102mC');
    const html = s.rowsHtml()[0];
    expect(html).toContain('color:#0a141e');
    expect(html).toContain('tf9');
    expect(html).toContain('tg10');
  });

  it('escapes what it prints so output is never markup', () => {
    const s = screen(2, 40);
    s.write('<img src=x onerror=alert(1)> & <b>');
    const html = s.rowsHtml()[0];
    expect(html).not.toContain('<img');
    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt; &amp; &lt;b&gt;');
  });

  it('colours a cleared area with the background in force', () => {
    const s = screen(2, 6);
    s.write('\x1b[44m\x1b[2J');
    expect(s.rowsHtml()[0]).toContain('tg4');
    expect(paletteHex(16)).toBe('#000000');
    expect(paletteHex(231)).toBe('#ffffff');
    expect(paletteHex(232)).toBe('#080808');
  });
});

describe('terminal screen: modes and the alternate screen', () => {
  it('tracks cursor keys, bracketed paste and cursor visibility', () => {
    const s = screen();
    s.write('\x1b[?1h\x1b[?2004h\x1b[?25l');
    expect([s.appCursorKeys, s.bracketedPaste, s.cursorVisible]).toEqual([true, true, false]);
    s.write('\x1b[?1l\x1b[?2004l\x1b[?25h');
    expect([s.appCursorKeys, s.bracketedPaste, s.cursorVisible]).toEqual([false, false, true]);
  });

  it('shows a full-screen program on the alternate screen and brings the shell back untouched', () => {
    const s = screen(3, 10);
    s.write('prompt$ ls\r\nfile');
    s.write('\x1b[?1049h\x1b[2J\x1b[1;1Hvim');
    expect(s.altScreen).toBe(true);
    expect(s.screenText()[0]).toBe('vim');
    for (let i = 0; i < 10; i++) s.write('\r\nfill');
    expect(s.scrollbackCount()).toBe(0); // the alternate screen keeps no scrollback
    s.write('\x1b[?1049l');
    expect(s.altScreen).toBe(false);
    expect(s.screenText().slice(0, 2)).toEqual(['prompt$ ls', 'file']);
    expect(s.cursorRow).toBe(1);
    expect(s.cursorCol).toBe(4);
  });

  it('uses the line-drawing set between ESC ( 0 and ESC ( B', () => {
    const s = screen(2, 10);
    s.write('\x1b(0lqk\x1b(Bq');
    expect(s.screenText()[0]).toBe('┌─┐q');
  });

  it('resets on RIS', () => {
    const s = screen(3, 6);
    s.write('abc\x1b[?1049h\x1b[?25l\x1bc');
    expect(s.altScreen).toBe(false);
    expect(s.cursorVisible).toBe(true);
    expect(s.screenText()).toEqual(['', '', '']);
  });
});

describe('terminal screen: what a program asks for and what it may not do', () => {
  it('answers a cursor position and a device attributes request', () => {
    const replies: string[] = [];
    const s = screen(5, 20, replies);
    s.write('\x1b[3;7H\x1b[6n\x1b[c\x1b[5n');
    expect(replies).toEqual(['\x1b[3;7R', '\x1b[?1;2c', '\x1b[0n']);
  });

  it('takes the window title and nothing else from OSC', () => {
    const s = screen();
    s.write('\x1b]0;my title\x07');
    expect(s.title).toBe('my title');
    s.write('\x1b]2;next\x1b\\');
    expect(s.title).toBe('next');
    // Clipboard (52), links (8) and file hints (7) are read and dropped.
    s.write('\x1b]52;c;aGVsbG8=\x07\x1b]8;;https://x.test\x07link\x1b]8;;\x07\x1b]7;file:///a\x07');
    expect(s.title).toBe('next');
    expect(s.screenText()[0]).toBe('link');
  });

  it('keeps control characters out of the title', () => {
    const s = screen();
    s.write('\x1b]0;a\x01b\x7fc\x07');
    expect(s.title).toBe('abc');
  });

  it('skips device control strings and unknown sequences to their end', () => {
    const s = screen();
    s.write('a\x1bP1$r0m\x1b\\b\x1b[?9999zc\x1b_apc\x07d\x1b[99;99;99ze');
    expect(s.screenText()[0]).toBe('abcde');
  });

  it('bounds an unterminated OSC and CSI', () => {
    const s = screen();
    s.write(`\x1b]0;${'x'.repeat(10000)}`);
    s.write('\x07ok');
    expect(s.title.length).toBeLessThanOrEqual(200);
    s.write(`\x1b[${'1;'.repeat(5000)}m`);
    s.write('fine');
    expect(s.screenText()[0]).toContain('fine');
  });
});

describe('terminal screen: size', () => {
  it('grows and shrinks, pushing the top rows into scrollback to keep the cursor line', () => {
    const s = screen(4, 10);
    s.write('1\r\n2\r\n3\r\n4');
    s.resize(2, 10);
    expect(s.rows).toBe(2);
    expect(s.screenText()).toEqual(['3', '4']);
    expect(s.allText().split('\n')).toEqual(['1', '2', '3', '4']);
    s.resize(5, 12);
    expect(s.rows).toBe(5);
    expect(s.cols).toBe(12);
    expect(s.screenText()).toHaveLength(5);
    expect(s.cursorRow).toBeLessThan(5);
  });

  it('bumps the version on every write and resize so the view re-reads', () => {
    const s = screen();
    const v = s.version;
    s.write('x');
    s.resize(6, 21);
    expect(s.version).toBeGreaterThan(v + 1);
  });
});
