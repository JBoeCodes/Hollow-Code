import { describe, expect, it } from 'vitest';
import { visualExplanationPrompt } from './visualExplain';

describe('Explain visually', () => {
  it('asks for an hc-flow canvas, never a whiteboard', () => {
    const prompt = visualExplanationPrompt('short');
    expect(prompt).toContain('hollow-canvas');
    expect(prompt).toContain('hc-flow');
    expect(prompt).not.toMatch(/whiteboard/i);
    // "canvas" is one of Hollow's visual cue words, so the turn brings the visual guide.
    expect(prompt.toLowerCase().split(/[^a-z]+/)).toContain('canvas');
  });

  it('bounds follow-up context and distinguishes truncation', () => {
    const prompt = visualExplanationPrompt('x'.repeat(20_000));
    expect(prompt.length).toBeLessThan(13_000);
    expect(prompt).toContain('truncated');
    expect(prompt).toContain('Do not change files');
    expect(visualExplanationPrompt('short')).not.toContain('truncated');
  });
});
