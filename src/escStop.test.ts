import { describe, expect, it } from 'vitest';
import { ESC_OWNER_SEL, escStopsRun, type EscFacts } from './escStop';

const esc = { key: 'Escape', defaultPrevented: false, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false };
const facts: EscFacts = { busy: true, inConversation: true, onComposer: true, onOtherField: false, ownerOpen: false, composerEmpty: true };

describe('Esc stops a running turn (frontier F6)', () => {
  it('stops from the composer or the transcript while a turn runs', () => {
    expect(escStopsRun(esc, facts)).toBe(true);
    expect(escStopsRun(esc, { ...facts, onComposer: false })).toBe(true);
  });
  it('stops only while the composer is empty (P1-27’s planning call)', () => {
    // A typed or attached message keeps the turn running, wherever focus is
    // in the conversation: Esc with a draft never throws the turn away.
    expect(escStopsRun(esc, { ...facts, composerEmpty: false })).toBe(false);
    expect(escStopsRun(esc, { ...facts, onComposer: false, composerEmpty: false })).toBe(false);
  });
  it('never steals Escape from a modal, menu, popup or the palette', () => {
    expect(escStopsRun(esc, { ...facts, ownerOpen: true })).toBe(false);
    // Resource menus and the usage dialog own Escape while open.
    for (const sel of ['.modal', '.palette-backdrop', '[role="menu"]', '[role="listbox"]', '.slash-popup', '.ctx-menu', '[role="dialog"]']) {
      expect(ESC_OWNER_SEL).toContain(sel);
    }
  });
  it('leaves another field’s Escape (a feedback reason) to that field', () => {
    expect(escStopsRun(esc, { ...facts, onComposer: false, onOtherField: true })).toBe(false);
  });
  it('does nothing when idle, outside the conversation, handled, repeated or chorded', () => {
    expect(escStopsRun(esc, { ...facts, busy: false })).toBe(false);
    expect(escStopsRun(esc, { ...facts, inConversation: false })).toBe(false);
    expect(escStopsRun({ ...esc, defaultPrevented: true }, facts)).toBe(false);
    expect(escStopsRun({ ...esc, repeat: true }, facts)).toBe(false);
    expect(escStopsRun({ ...esc, isComposing: true }, facts)).toBe(false);
    expect(escStopsRun({ ...esc, metaKey: true }, facts)).toBe(false);
    expect(escStopsRun({ ...esc, key: 'x' }, facts)).toBe(false);
  });
});
