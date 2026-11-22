import { describe, expect, it } from 'vitest';
import { CALENDAR_SOURCE, KEPT_COPIES, MEETINGS_ACCESS, PLAUD_FOR_AGENTS, RECORDINGS_IN_HOLLOW } from './meetingsCopy';
import { SOURCE_COLLECTIONS } from './sourceCatalog';
import settings from './pages/Settings.tsx?raw';
import brief from '../src-tauri/src/chat.rs?raw';

// The app's source files, to show no copy still tells the old meetings story.
const sources = import.meta.glob(['./**/*.ts', './**/*.tsx', '!./**/*.test.ts', '!./**/*.test.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

// P1-20: one meetings paragraph names the two Plaud accesses and a
// provider-agnostic calendar line, in Settings, Library and the agent brief.
describe('meetings and recordings copy (P1-20)', () => {
  it('names both Plaud accesses and where calendar comes from', () => {
    expect(RECORDINGS_IN_HOLLOW).toMatch(/^Recordings in Hollow /);
    expect(RECORDINGS_IN_HOLLOW).toContain('Keep a transcript copy');
    expect(PLAUD_FOR_AGENTS).toMatch(/^Plaud tools for agents /);
    expect(PLAUD_FOR_AGENTS).toContain('its own sign-in');
    expect(CALENDAR_SOURCE).toBe('Calendar comes from the source you connect in Settings → Connectors.');
    expect(MEETINGS_ACCESS).toBe(`${RECORDINGS_IN_HOLLOW} ${PLAUD_FOR_AGENTS} ${CALENDAR_SOURCE}`);
  });
  it('is the Library collections’ wording; Settings’ Recordings subhead is one line (Settings PR 6)', () => {
    expect(settings).toContain('<SettingsSubhead title="Recordings" hint="Plaud recordings and your calendar, separate from agent access." />');
    const of = (id: string) => SOURCE_COLLECTIONS.find(c => c.id === id)!.description;
    expect(of('calendar')).toContain(CALENDAR_SOURCE);
    expect(of('recordings')).toContain(KEPT_COPIES);
    expect(KEPT_COPIES).toContain('Keep a transcript copy');
  });
  it('is what agents hear in the brief', () => {
    expect(brief).toContain(`${PLAUD_FOR_AGENTS} ${CALENDAR_SOURCE}`);
  });
  it('leaves none of the old meetings wording in the app', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(100);
    for (const [file, text] of Object.entries(sources))
      for (const stale of ['live in Notion', 'keeps no copies', 'no longer a built-in connection', 'plaud me'])
        expect(text.includes(stale), `${file}: ${stale}`).toBe(false);
  });
});
