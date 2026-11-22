import { describe, expect, it } from 'vitest';
import GOLDEN from './answerLines.golden.json?raw';
import BLOCKS from './answerBlocks.golden.json?raw';
import PHONE_REQUEST from '../../docs/phone-api-fixtures/canvases/answer-request.json?raw';
import PHONE_MESSAGE from '../../docs/phone-api-fixtures/canvases/answer-message.txt?raw';
import {
  ANSWER_FENCE, answerKey, answerLine, answeredCanvases, canvasAnswerText, readCanvasAnswers, visibleUserText,
  type CanvasAnswer,
} from './askAnswer';
import { LIMITS } from '../canvasBridge';

const golden = JSON.parse(GOLDEN) as { cases: Array<{ label: string; value: string; line: string }> };
const blocks = JSON.parse(BLOCKS) as { cases: Array<{ name: string; text: string; answered: string[] }> };

const ANSWER: CanvasAnswer = {
  canvasId: 'release-choices',
  revision: 2,
  answers: { 'ship-day': 'Friday, after the bed passes', checks: ['Bed rows', 'Bundle budget'], budget: 24, fit: { Speed: 'High', Cost: 'Low' }, notes: null },
};
const SUMMARY = [
  'When should the release ship?: Friday, after the bed passes',
  'Which checks must pass first?: Bed rows, Bundle budget',
  'Kit budget: 24 KB',
  'Rate each option: Speed: High; Cost: Low',
].join('\n');

describe('answer lines (shared with Rust push_line)', () => {
  it('every golden case formats exactly as format_answers does', () => {
    expect(golden.cases.length).toBeGreaterThan(10);
    for (const c of golden.cases) expect(answerLine(c.label, c.value), JSON.stringify(c.label).slice(0, 40)).toBe(c.line);
  });
});

describe('canvasAnswerText: what a draft puts in the composer', () => {
  it('is the header, one "- label: value" line per summary line, then the exact JSON in a hidden block', () => {
    const text = canvasAnswerText(ANSWER, SUMMARY);
    expect(text).toBe([
      'Answered canvas "release-choices":',
      '- When should the release ship?: Friday, after the bed passes',
      '- Which checks must pass first?: Bed rows, Bundle budget',
      '- Kit budget: 24 KB',
      '- Rate each option: Speed: High; Cost: Low',
      '',
      '```hollow-canvas-answer',
      '{"canvasId":"release-choices","revision":2,"answers":{"ship-day":"Friday, after the bed passes","checks":["Bed rows","Bundle budget"],"budget":24,"fit":{"Speed":"High","Cost":"Low"},"notes":null}}',
      '```',
    ].join('\n'));
    expect(ANSWER_FENCE).toBe('hollow-canvas-answer');
  });

  it('a summary line with no "label: value" stays one line; blank lines are dropped', () => {
    expect(canvasAnswerText({ ...ANSWER, answers: {} }, 'No answers given.\n\n').split('\n').slice(0, 3))
      .toEqual(['Answered canvas "release-choices":', '- No answers given.', '']);
  });

  it('keeps the JSON on one line whatever the answers hold, so it can never close its own fence', () => {
    const tricky: CanvasAnswer = { ...ANSWER, answers: { notes: 'line one\n```\nline two' } };
    const text = canvasAnswerText(tricky, 'Notes: line one ``` line two');
    expect(text.split('\n').filter(l => l.startsWith('```'))).toEqual(['```hollow-canvas-answer', '```']);
    expect(readCanvasAnswers(text)).toEqual([tricky]);
  });
});

describe('reading answers back from a sent message', () => {
  it('round-trips: the composer text reads back as the exact answer', () => {
    expect(readCanvasAnswers(canvasAnswerText(ANSWER, SUMMARY))).toEqual([ANSWER]);
  });

  it('the visible text leaves the hidden block out (closed, or left open at the end)', () => {
    const text = canvasAnswerText(ANSWER, SUMMARY);
    expect(visibleUserText(text)).toBe(text.slice(0, text.indexOf('\n\n```')));
    expect(visibleUserText('Hi\n```hollow-canvas-answer\n{"canvasId":"a"')).toBe('Hi');
    expect(visibleUserText('plain words')).toBe('plain words');
    // Any other fence stays, so one quoting the block is shown.
    const quoted = 'See:\n````md\n```hollow-canvas-answer\n{}\n```\n````';
    expect(visibleUserText(quoted)).toBe(quoted);
    expect(readCanvasAnswers(quoted)).toEqual([]);
  });

  it('a block that is not exactly {canvasId, revision, answers} of the draft shape is no answer', () => {
    const block = (json: string) => `Answered\n\n\`\`\`hollow-canvas-answer\n${json}\n\`\`\``;
    for (const json of [
      '', 'not json', '[]', 'null',
      '{"canvasId":"a","revision":1}',
      '{"canvasId":"a","revision":1,"answers":{},"extra":1}',
      '{"canvasId":"A","revision":1,"answers":{}}',
      '{"canvasId":"a","revision":0,"answers":{}}',
      '{"canvasId":"a","revision":1.5,"answers":{}}',
      '{"canvasId":"a","revision":"1","answers":{}}',
      '{"canvasId":"a","revision":1,"answers":{"x":{"deep":{"er":"y"}}}}',
      '{"canvasId":"a","revision":1,"answers":{"bad name":"y"}}',
    ]) expect(readCanvasAnswers(block(json)), json).toEqual([]);
    expect(readCanvasAnswers(block('{"canvasId":"a","revision":1,"answers":{}}'))).toEqual([{ canvasId: 'a', revision: 1, answers: {} }]);
  });
});

describe('answer blocks (shared with Rust canvas_answers::answered, C4 A3)', () => {
  it('every golden message answers exactly the canvas revisions it lists, so "waiting on you" and the lock agree', () => {
    expect(blocks.cases.length).toBeGreaterThan(15);
    for (const c of blocks.cases) {
      expect(readCanvasAnswers(c.text).map(a => answerKey(a.canvasId, a.revision)), c.name).toEqual(c.answered);
    }
  });
});

describe('answeredCanvases: the lock, read from the transcript', () => {
  it('does not lock an optimistic local answer before queue acceptance or run evidence', () => {
    const pending = { role: 'user', text: canvasAnswerText(ANSWER, SUMMARY), clientTurnId: 'pending-answer' };
    expect(answeredCanvases([pending]).size).toBe(0);
    expect(answeredCanvases([{ ...pending, queueId: 42 }]).size).toBe(1);
    expect(answeredCanvases([{ ...pending, runId: 'started-run' }]).size).toBe(1);
    expect(answeredCanvases([{ ...pending, status: 'success' }]).size).toBe(1);
    expect(answeredCanvases([{ role: 'user', text: pending.text }]).size).toBe(1);
  });
  const canvasReply = { role: 'assistant', text: 'Pick one.\n\n```hollow-canvas\n<hc-canvas id="release-choices" title="Release" ask>…</hc-canvas>\n```' };

  it('a sent answer locks that canvas revision and keeps what the person sent', () => {
    const sent = canvasAnswerText(ANSWER, SUMMARY);
    const locks = answeredCanvases([{ role: 'user', text: 'Plan the release' }, canvasReply, { role: 'user', text: sent }]);
    expect([...locks.keys()]).toEqual([answerKey('release-choices', 2)]);
    const lock = locks.get('release-choices@2')!;
    expect(lock.answer).toEqual(ANSWER);
    expect(lock.turn).toBe(2);
    // The summary is the visible lines the person sent, without the header.
    expect(lock.summary).toBe(SUMMARY.split('\n').map(l => `- ${l}`).join('\n'));
  });

  it('sending something else first leaves it open; only a user message locks', () => {
    expect(answeredCanvases([canvasReply, { role: 'user', text: 'Actually, wait.' }]).size).toBe(0);
    expect(answeredCanvases([canvasReply, { role: 'assistant', text: canvasAnswerText(ANSWER, SUMMARY) }]).size).toBe(0);
  });

  it('a later revision of the same canvas is a new question', () => {
    const locks = answeredCanvases([canvasReply, { role: 'user', text: canvasAnswerText(ANSWER, SUMMARY) }]);
    expect(locks.has(answerKey('release-choices', 2))).toBe(true);
    expect(locks.has(answerKey('release-choices', 3))).toBe(false);
  });

  it('the first answer to a revision is the one it shows; the person\'s own edits to the lines show too', () => {
    const edited = canvasAnswerText(ANSWER, SUMMARY).replace('- Kit budget: 24 KB', '- Kit budget: 24 KB (or less)');
    const again = canvasAnswerText({ ...ANSWER, answers: { budget: 30 } }, 'Kit budget: 30 KB');
    const lock = answeredCanvases([canvasReply, { role: 'user', text: edited }, { role: 'user', text: again }]).get('release-choices@2')!;
    expect(lock.turn).toBe(1);
    expect(lock.summary).toContain('24 KB (or less)');
    expect(lock.answer.answers.budget).toBe(24);
  });

  it('the summary a locked frame gets is clipped to the bridge limit', () => {
    const long = canvasAnswerText(ANSWER, Array.from({ length: 40 }, (_, i) => `Question ${i}: ${'answer '.repeat(12)}`).join('\n'));
    const lock = answeredCanvases([{ role: 'user', text: long }]).get('release-choices@2')!;
    expect(lock.summary.length).toBeLessThanOrEqual(LIMITS.answeredChars);
  });

  // Phone API v1.2 (docs/phone-api/canvases.md): Hollow writes an iPhone's
  // ask-canvas answer in Rust (`phone_access::canvases::answer_text`), held
  // to the same fixture message as this window's Send.
  it('the phone answer fixture is the message this window sends', () => {
    const req = JSON.parse(PHONE_REQUEST) as { revision: number; answers: CanvasAnswer['answers']; summary: string };
    const text = canvasAnswerText({ canvasId: 'launch-order', revision: req.revision, answers: req.answers }, req.summary);
    expect(text).toBe(PHONE_MESSAGE.trimEnd());
    expect(readCanvasAnswers(text)).toEqual([{ canvasId: 'launch-order', revision: req.revision, answers: req.answers }]);
  });
});
