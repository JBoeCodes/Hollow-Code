import { describe, expect, it } from 'vitest';
import {
  CANVAS_FREQUENCIES, CANVAS_FREQUENCY_LABELS, canvasesForSave, canvasesMenuValue, chatCanvasesOnKeyChange,
  chatCanvasesStorageKey, effectiveCanvases, parseCanvasFrequency, settingCanvases,
} from './canvasFrequency';

describe('Canvases choice (Canvas C3 wave 2 C4)', () => {
  it('names the three choices in order, as the backend spells them', () => {
    expect(CANVAS_FREQUENCIES).toEqual(['off', 'helps', 'most']);
    expect(CANVAS_FREQUENCIES.map(c => CANVAS_FREQUENCY_LABELS[c])).toEqual(['Off', 'When it helps', 'Most replies']);
  });

  it('reads the setting with When it helps as the default', () => {
    expect(settingCanvases(undefined)).toBe('helps');
    expect(settingCanvases({})).toBe('helps');
    expect(settingCanvases({ canvases: 'off' })).toBe('off');
    expect(settingCanvases({ canvases: 'always' })).toBe('helps');
    expect(parseCanvasFrequency('most')).toBe('most');
    expect(parseCanvasFrequency(' most')).toBeNull();
    expect(parseCanvasFrequency(3)).toBeNull();
    expect(canvasesForSave('helps')).toBeUndefined();
    expect(canvasesForSave(null)).toBeUndefined();
    expect(canvasesForSave('off')).toBe('off');
  });

  it('a chat\'s own choice wins over the setting', () => {
    expect(effectiveCanvases(null, 'off')).toBe('off');
    expect(effectiveCanvases('most', 'off')).toBe('most');
    expect(canvasesMenuValue(null, 'helps')).toBe('When it helps · setting');
    expect(canvasesMenuValue('off', 'most')).toBe('Off');
  });

  it('travels with the conversation', () => {
    expect(chatCanvasesStorageKey('s-1')).toBe('hollow.canvases.s-1');
    // A new chat keeps what was chosen before its first send, under its new key.
    expect(chatCanvasesOnKeyChange(null, 's-new', 'most', null)).toEqual({ choice: 'most', persist: true });
    // A new chat with no choice reads nothing.
    expect(chatCanvasesOnKeyChange(null, 's-new', null, null)).toEqual({ choice: null, persist: false });
    // Opening a stored conversation reads its own choice.
    expect(chatCanvasesOnKeyChange(null, 's-old', null, 'off')).toEqual({ choice: 'off', persist: false });
    expect(chatCanvasesOnKeyChange('s-a', 's-b', 'most', 'off')).toEqual({ choice: 'off', persist: false });
    // Switching to a conversation that never chose follows the setting.
    expect(chatCanvasesOnKeyChange('s-a', 's-b', 'most', null)).toEqual({ choice: null, persist: false });
    // Leaving for a fresh chat clears it; the same key keeps it.
    expect(chatCanvasesOnKeyChange('s-a', null, 'most', null)).toEqual({ choice: null, persist: false });
    expect(chatCanvasesOnKeyChange('s-a', 's-a', 'most', 'off')).toEqual({ choice: 'most', persist: false });
  });
});
