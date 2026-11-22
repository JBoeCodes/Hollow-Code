import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readAloud, readAloudSupported, readingKey, speakableText, speechChunks, stopReading } from './readAloud';

// Read aloud on a reply (overnight 11-07, D1): the Markdown made speakable,
// cut into spoken pieces, and the one-reply-at-a-time engine over a fake
// `speechSynthesis`.
describe('speakableText', () => {
  it('drops the markup and keeps the words', () => {
    const md = ['# The plan', '', 'Use **Fly** for the *API*, see [the docs](https://fly.io/docs) or `fly deploy`.', '', '> A quoted line', '', '---'].join('\n');
    expect(speakableText(md)).toBe('The plan.\n\nUse Fly for the API, see the docs or fly deploy.\n\nA quoted line');
  });

  it('reads a list item and a table row as their own sentences', () => {
    const md = ['- first', '- second one.', '3. third', '- [x] done item', '', '| Option | Cost |', '|---|---:|', '| Fly | $12 |'].join('\n');
    expect(speakableText(md)).toBe('first.\nsecond one.\nthird.\ndone item.\n\nOption, Cost.\nFly, $12.');
  });

  it('says a code block was skipped instead of reading it', () => {
    const md = ['Run this:', '', '```bash', 'rm -rf /tmp/x', '```', '', 'Then ~~~', '', '~~~', 'text in a tilde fence', '~~~', 'Done'].join('\n');
    expect(speakableText(md)).toBe('Run this:\n\nCode block skipped.\n\nThen ~~~\n\nCode block skipped.\nDone');
  });

  it('keeps a longer fence open until its own marker and skips an unclosed one', () => {
    expect(speakableText(['````md', '```', 'inner', '```', '````', 'after'].join('\n'))).toBe('Code block skipped.\nafter');
    expect(speakableText(['before', '```', 'never closed'].join('\n'))).toBe('before\nCode block skipped.');
  });

  it('reads an address as "link" and an image as its description', () => {
    expect(speakableText('Open https://example.com/a?b=1 or <https://example.com/b> now. ![A chart of costs](chart.png)')).toBe('Open link or link now. A chart of costs');
  });

  it('does not end a wrapped paragraph line like a sentence', () => {
    expect(speakableText('The quick brown fox\njumped over the dog.')).toBe('The quick brown fox\njumped over the dog.');
  });

  it('leaves snake_case and times alone', () => {
    expect(speakableText('Call my_var_name at 3.5 seconds, a*b*c stays')).toBe('Call my_var_name at 3.5 seconds, a*b*c stays');
  });

  it('is empty for markup with no words', () => {
    expect(speakableText('---\n\n***\n')).toBe('');
    expect(speakableText('   ')).toBe('');
  });

  // The Grok review of #1195: each of these was read wrongly.
  it('keeps the punctuation after an address', () => {
    expect(speakableText('Read https://example.com/docs. Then stop. (see https://x.com/a) or https://y.com/b, ok')).toBe('Read link. Then stop. (see link) or link, ok');
  });

  it('leaves inline code as typed, whatever it looks like', () => {
    expect(speakableText('Use `items[i](value)` and `<div>` and `https://x.com` here.')).toBe('Use items[i](value) and <div> and https://x.com here.');
  });

  it('reads a year before a full stop as a year, a short number as a list number', () => {
    expect(speakableText('2024. Revenue grew.')).toBe('2024. Revenue grew.');
    expect(speakableText('12. twelfth item')).toBe('twelfth item.');
  });

  it('leaves arithmetic asterisks alone and still drops emphasis', () => {
    expect(speakableText('2 * 3 * 4 = 24')).toBe('2 * 3 * 4 = 24');
    expect(speakableText('It is *very* clear and **bold**')).toBe('It is very clear and bold');
  });

  it('does not open a fence on a line that closes itself', () => {
    expect(speakableText(['```inline```', 'This paragraph should be read.'].join('\n'))).toBe('inline\nThis paragraph should be read.');
  });

  it('skips a fence under a list item, however far it is indented', () => {
    expect(speakableText(['10. Run:', '    ```bash', '    rm -rf /tmp/x', '    ```', 'after'].join('\n'))).toBe('Run:\nCode block skipped.\nafter');
    // Across a blank line, and a longer indent (two levels of list).
    expect(speakableText(['- Outer', '', '  - Inner', '', '        ```sh', '        ls', '        ```', 'after'].join('\n'))).toBe('Outer.\n\nInner.\n\nCode block skipped.\nafter');
  });

  it('ends a fence under a list item with the item, never swallowing the prose after it', () => {
    expect(speakableText(['1. Run:', '    ```sh', '    ls', '', 'This paragraph must be spoken.'].join('\n'))).toBe('Run:\nCode block skipped.\nThis paragraph must be spoken.');
  });

  it('keeps a fence under a list item open on lines indented like the item text, even left of its opener', () => {
    expect(speakableText(['1. a', '    ```sh', '   ls', '    ```', 'b'].join('\n'))).toBe('a.\nCode block skipped.\nb');
    expect(speakableText(['1. a', '    ~~~', '   ls', '    ~~~', 'b'].join('\n'))).toBe('a.\nCode block skipped.\nb');
    // A fence at the margin after a list is its own, not the item's.
    expect(speakableText(['1. a', '```sh', 'ls', '```', 'b'].join('\n'))).toBe('a.\nCode block skipped.\nb');
  });

  it('does not take four spaces outside a list for a fence, so the prose after it is kept', () => {
    expect(speakableText(['    ```js', '    const n = 1;', '', 'This paragraph must be spoken.'].join('\n'))).toBe('```js\nconst n = 1;\n\nThis paragraph must be spoken.');
  });

  it('reads "1000)" as a list item but a year with a full stop as a sentence', () => {
    expect(speakableText('1000) First item\n1001) Second item')).toBe('First item.\nSecond item.');
    expect(speakableText('2024. Revenue grew.')).toBe('2024. Revenue grew.');
  });
});

describe('speechChunks', () => {
  it('cuts after each sentence and joins short ones into one piece', () => {
    expect(speechChunks('One. Two! Three?\nFour.')).toEqual(['One. Two! Three? Four.']);
  });

  it('keeps a decimal number whole', () => {
    expect(speechChunks('It took 3.5 seconds. Then it stopped.')).toEqual(['It took 3.5 seconds. Then it stopped.']);
  });

  it('never exceeds the limit and never splits a word', () => {
    const sentence = Array.from({ length: 60 }, (_, i) => `word${i}`).join(' ');
    const chunks = speechChunks(`${sentence}.`, 100);
    expect(chunks.length).toBeGreaterThan(3);
    for (const c of chunks) expect(c.length).toBeLessThanOrEqual(100);
    expect(chunks.join(' ')).toBe(`${sentence}.`);
  });

  it('prefers a comma to a space when one is near the end', () => {
    const text = `${'a'.repeat(60)}, ${'b'.repeat(60)} ${'c'.repeat(60)}`;
    expect(speechChunks(text, 100)[0]).toBe(`${'a'.repeat(60)},`);
  });

  it('cuts one endless token at the limit', () => {
    expect(speechChunks('x'.repeat(250), 100).map(c => c.length)).toEqual([100, 100, 50]);
  });

  it('is empty for nothing', () => {
    expect(speechChunks('')).toEqual([]);
    expect(speechChunks('\n\n')).toEqual([]);
  });
});

/** A `speechSynthesis` that records what it was told, and says whether it is
 *  speaking the way a voice does (`speaking` from the first piece to the last). */
function fakeSpeech() {
  type Utter = { text: string; onend?: () => void; onerror?: (e: { error: string }) => void };
  const spoken: { text: string; u: Utter }[] = [];
  const synth = {
    speaking: false,
    pending: false,
    speak: vi.fn((u: Utter) => { spoken.push({ text: u.text, u }); synth.speaking = true; }),
    cancel: vi.fn(() => { spoken.length = 0; synth.speaking = false; synth.pending = false; }),
  };
  class Utterance { text: string; lang = ''; onend?: () => void; onerror?: (e: { error: string }) => void; constructor(text: string) { this.text = text; } }
  // The suite runs in Node: the webview's `window` is a stand-in with the one property read.
  vi.stubGlobal('window', { speechSynthesis: synth });
  vi.stubGlobal('SpeechSynthesisUtterance', Utterance);
  return { synth, spoken };
}

describe('the read aloud engine', () => {
  beforeEach(() => { stopReading(); });
  afterEach(() => { stopReading(); vi.useRealTimers(); vi.unstubAllGlobals(); });

  it('needs the system voice', () => {
    expect(readAloudSupported()).toBe(false);
    expect(readAloud('c:1', 'Hello there.')).toBe(false);
    expect(readingKey()).toBeNull();
    fakeSpeech();
    expect(readAloudSupported()).toBe(true);
  });

  it('speaks a reply in pieces, names it while it reads and clears when the last piece ends', () => {
    const { synth, spoken } = fakeSpeech();
    const long = Array.from({ length: 40 }, (_, i) => `Sentence number ${i} says something.`).join(' ');
    expect(readAloud('c:1', `# Title\n\n${long}`)).toBe(true);
    expect(readingKey()).toBe('c:1');
    // Nothing was speaking, so nothing is cancelled and the pieces go at once.
    expect(synth.cancel).not.toHaveBeenCalled();
    expect(spoken.length).toBeGreaterThan(2);
    expect(spoken[0].text.startsWith('Title.')).toBe(true);
    // Earlier pieces ending change nothing; the last one ends the read.
    spoken.slice(0, -1).forEach(s => s.u.onend?.());
    expect(readingKey()).toBe('c:1');
    spoken.at(-1)!.u.onend!();
    expect(readingKey()).toBeNull();
  });

  it('stops at once and ignores the cancel it causes', () => {
    const { synth, spoken } = fakeSpeech();
    readAloud('c:1', 'One short reply.');
    const first = spoken[0].u;
    stopReading();
    expect(readingKey()).toBeNull();
    expect(synth.cancel).toHaveBeenCalledTimes(1);
    first.onerror?.({ error: 'canceled' });
    expect(readingKey()).toBeNull();
  });

  it('lets a view stop only the read it started: another view going away leaves it alone', () => {
    vi.useFakeTimers();
    fakeSpeech();
    const a = {};
    const b = {};
    readAloud('c:1', 'One short reply.', a);
    // View b closes (the popover, a hidden page): the read a started goes on.
    stopReading(b);
    expect(readingKey()).toBe('c:1');
    stopReading(a);
    expect(readingKey()).toBeNull();
    // A read b took over from a is no longer a's to stop; a stop with no view always stops.
    readAloud('c:1', 'One short reply.', a);
    readAloud('c:2', 'Another short reply.', b);
    stopReading(a);
    expect(readingKey()).toBe('c:2');
    stopReading();
    expect(readingKey()).toBeNull();
  });

  it('reads one reply at a time: the next read cancels the voice, waits a moment, then speaks', () => {
    vi.useFakeTimers();
    const { synth, spoken } = fakeSpeech();
    readAloud('c:1', 'First reply.');
    const old = spoken[0].u;
    readAloud('c:2', 'Second reply.');
    // The row switches at once; WebKit only gets the new pieces after the cancel has settled.
    expect(readingKey()).toBe('c:2');
    expect(synth.cancel).toHaveBeenCalledTimes(1);
    expect(spoken).toEqual([]);
    vi.advanceTimersByTime(80);
    expect(spoken.map(s => s.text)).toEqual(['Second reply.']);
    old.onend?.();
    old.onerror?.({ error: 'interrupted' });
    expect(readingKey()).toBe('c:2');
    spoken.at(-1)!.u.onend!();
    expect(readingKey()).toBeNull();
  });

  it('speaks nothing when it is stopped, or taken over, during that moment', () => {
    vi.useFakeTimers();
    const { spoken } = fakeSpeech();
    readAloud('c:1', 'First reply.');
    readAloud('c:2', 'Second reply.');
    stopReading();
    vi.advanceTimersByTime(200);
    expect(spoken).toEqual([]);
    expect(readingKey()).toBeNull();
    readAloud('c:3', 'Third reply.');
    readAloud('c:4', 'Fourth reply.');
    readAloud('c:5', 'Fifth reply.');
    vi.advanceTimersByTime(200);
    expect(spoken.map(s => s.text)).toEqual(['Fifth reply.']);
    expect(readingKey()).toBe('c:5');
  });

  // WebKit 238189: the last piece may never report its end.
  it('ends by itself when the voice goes quiet without an end event', () => {
    vi.useFakeTimers();
    const { synth } = fakeSpeech();
    readAloud('c:1', 'A reply whose end is never reported.');
    expect(readingKey()).toBe('c:1');
    synth.speaking = false;
    vi.advanceTimersByTime(800);
    expect(readingKey()).toBe('c:1');
    vi.advanceTimersByTime(600);
    expect(readingKey()).toBeNull();
  });

  it('keeps reading while the voice is speaking or has pieces queued, however long', () => {
    vi.useFakeTimers();
    const { synth } = fakeSpeech();
    readAloud('c:1', 'A long reply.');
    vi.advanceTimersByTime(30_000);
    expect(readingKey()).toBe('c:1');
    // A gap between two pieces (nothing speaking, the next one queued) is not the end.
    synth.speaking = false;
    synth.pending = true;
    vi.advanceTimersByTime(30_000);
    expect(readingKey()).toBe('c:1');
  });

  it('says so and stops when the voice fails', () => {
    const { synth, spoken } = fakeSpeech();
    readAloud('c:1', 'A reply.');
    spoken[0].u.onerror?.({ error: 'synthesis-failed' });
    expect(readingKey()).toBeNull();
    expect(synth.cancel).toHaveBeenCalledTimes(1);
  });

  it('has nothing to say for a reply that is only code', () => {
    const { synth } = fakeSpeech();
    // "Code block skipped." is spoken: the reply is not silent.
    expect(readAloud('c:1', '```\nx\n```')).toBe(true);
    expect(synth.speak).toHaveBeenCalledTimes(1);
    stopReading();
    expect(readAloud('c:2', '---')).toBe(false);
    expect(readingKey()).toBeNull();
  });
});
