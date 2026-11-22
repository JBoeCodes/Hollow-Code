import { describe, expect, it } from 'vitest';
import { annotationText, describeElement, markAnchor, normalizeBox, toViewport, type Mark, type PageElement } from './annotate';

const button: PageElement = { tag: 'button', id: '', classes: 'btn primary extra', text: 'Refresh view', label: '', href: '', selector: 'main > header > button.btn.primary' };

describe('annotate geometry', () => {
  it('anchors each kind of mark', () => {
    expect(markAnchor({ kind: 'pin', x: 10, y: 20, note: '' })).toEqual({ x: 10, y: 20 });
    expect(markAnchor({ kind: 'box', x: 10, y: 20, w: 100, h: 40, note: '' })).toEqual({ x: 60, y: 40 });
    expect(markAnchor({ kind: 'draw', points: [[5, 6], [9, 9]], note: '' })).toEqual({ x: 5, y: 6 });
  });
  it('normalizes a box dragged up and left', () => {
    expect(normalizeBox(100, 80, 40, 20)).toEqual({ x: 40, y: 20, w: 60, h: 60 });
  });
  it('maps snapshot pixels to the viewport', () => {
    expect(toViewport({ x: 200, y: 100 }, { width: 1200, height: 1600 }, { width: 600, height: 800 })).toEqual([100, 50]);
  });
});

describe('annotation text', () => {
  it('whole-page comments never borrow a neighboring inspection element', () => {
    const marks = [
      { kind: 'page', note: 'Whole page' },
      { kind: 'pin', x: 8, y: 16, note: 'One button' },
    ] as Mark[];
    const text = annotationText({ url: 'http://localhost/', title: 'Page' }, marks, {
      url: 'http://localhost/', title: 'Page', elements: [null, button],
    });
    expect(text).toContain('1. Whole page: Whole page\n2. One button — on <button.btn.primary>');
  });
  it('describes an element briefly', () => {
    expect(describeElement(button)).toBe('<button.btn.primary> "Refresh view" (`main > header > button.btn.primary`)');
    expect(describeElement({ ...button, id: 'go', label: 'Go now', href: 'https://x.test/a', selector: 'a' , tag: 'a'})).toBe('<a#go.btn.primary> "Go now" → https://x.test/a');
    expect(describeElement(null)).toBe('');
  });
  it('numbers every mark with its note and element', () => {
    const marks: Mark[] = [
      { kind: 'pin', x: 1, y: 1, note: ' Make this blue ' },
      { kind: 'box', x: 0, y: 0, w: 5, h: 5, note: '' },
      { kind: 'draw', points: [[0, 0], [1, 1]], note: 'this gap' },
    ];
    const text = annotationText({ url: 'http://localhost:5173/', title: 'Tab title' }, marks, {
      url: 'http://localhost:5173/pricing', title: 'Pricing', elements: [button, null, button],
    });
    expect(text).toBe([
      'I marked up “Pricing” (http://localhost:5173/pricing); the annotated screenshot is attached.',
      '',
      '1. Make this blue — on <button.btn.primary> "Refresh view" (`main > header > button.btn.primary`)',
      '2. Box: (no comment)',
      '3. Drawing: this gap',
    ].join('\n'));
  });
  it('falls back to the tab when the page could not be asked', () => {
    expect(annotationText({ url: 'https://x.test/', title: '' }, [], null)).toBe('I marked up https://x.test/; the annotated screenshot is attached.');
  });
});
