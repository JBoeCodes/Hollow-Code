import { describe, expect, it } from 'vitest';
import { parseLocalDateTime, resumeAtLabel, resumeAtWhen, resumePresets, toLocalDateTime } from './chatResumeAt';
import CONTROL from './components/chat/ResumeAtControl.tsx?raw';
import MISSION from './components/MissionControl.tsx?raw';
import COMPOSER from './components/ChatBody.tsx?raw';

// P1c: the "Resume at..." presets and how a resume time reads. Dates are
// built in the machine's zone, as the module reads them.
const at = (y: number, mo: number, d: number, h: number, mi = 0) => new Date(y, mo - 1, d, h, mi).getTime();
const secs = (ms: number) => Math.floor(ms / 1000);

describe('resumePresets', () => {
  it('offers in 1 hour, tonight 9 PM and tomorrow 8 AM in the afternoon', () => {
    const now = at(2026, 11, 15, 14, 30);
    expect(resumePresets(now)).toEqual([
      { id: 'hour', label: 'In 1 hour', at: secs(now) + 3600 },
      { id: 'tonight', label: 'Tonight, 9 PM', at: secs(at(2026, 11, 15, 21)) },
      { id: 'morning', label: 'Tomorrow, 8 AM', at: secs(at(2026, 11, 16, 8)) },
    ]);
  });
  it('drops tonight once 9 PM has passed, and tomorrow crosses a month end', () => {
    const now = at(2026, 11, 30, 22, 15);
    expect(resumePresets(now).map(p => p.id)).toEqual(['hour', 'morning']);
    expect(resumePresets(now).at(-1)!.at).toBe(secs(at(2026, 12, 1, 8)));
  });
});

describe('resumeAtWhen', () => {
  const now = at(2026, 11, 15, 14, 30);
  it('reads today, tomorrow, this week and later', () => {
    expect(resumeAtWhen(secs(at(2026, 11, 15, 21)), now)).toBe('at 21:00');
    expect(resumeAtWhen(secs(at(2026, 11, 16, 8)), now)).toBe('tomorrow at 08:00');
    expect(resumeAtWhen(secs(at(2026, 11, 20, 8, 5)), now)).toBe('Fri at 08:05');
    expect(resumeAtWhen(secs(at(2026, 12, 2, 9)), now)).toBe('Dec 2 at 09:00');
    expect(resumeAtWhen(secs(now) - 5, now)).toBe('now');
    expect(resumeAtLabel(secs(at(2026, 11, 16, 8)), now)).toBe('Resumes tomorrow at 08:00');
  });
});

describe('the custom time', () => {
  it('round-trips a datetime-local value and refuses anything else', () => {
    const t = secs(at(2026, 11, 16, 8, 45));
    expect(toLocalDateTime(t)).toBe('2026-11-16T08:45');
    expect(parseLocalDateTime('2026-11-16T08:45')).toBe(t);
    expect(parseLocalDateTime('')).toBeNull();
    expect(parseLocalDateTime('2026-02-30T08:00')).toBeNull();
    expect(parseLocalDateTime('tomorrow')).toBeNull();
  });
});

// Every control has a visible label and a click path (P1b, P1c).
describe('the controls', () => {
  const control = CONTROL;
  const mission = MISSION;
  const composer = COMPOSER;
  it('Resume at… is a disclosure with a labelled group, presets, a custom time and Clear time', () => {
    expect(control).toContain('aria-expanded={open()}');
    expect(control).toContain('role="group" aria-label="Resume at"');
    expect(control).toContain('<span>Custom time</span>');
    expect(control).toContain('type="datetime-local"');
    expect(control).toMatch(/>Set time</);
    expect(control).toMatch(/>Clear time</);
    expect(control).toContain('role="alert"');
  });
  it('the composer and Running offer it on a paused chat, and Running has Pause all and Resume all', () => {
    expect(composer).toContain("lazy(() => import('./chat/ResumeAtControl'))");
    expect(composer).toMatch(/<ResumeAtControl sessionKey=\{chatAgentSessionKey\(\)!\}/);
    expect(mission).toMatch(/>Pause all</);
    expect(mission).toMatch(/>Resume all</);
    expect(mission.match(/<ResumeAtControl /g)?.length).toBe(2);
  });
});
