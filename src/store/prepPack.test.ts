import { describe, expect, it } from 'vitest';
import { packBody } from './prepPack';
import recordingsSource from '../components/RecordingsView.tsx?raw';

describe('meeting prep packs (overnight 10-11 A4)', () => {
  it('reads the page without its frontmatter', () => {
    expect(packBody('---\ntype: prep-pack\nmeeting: "x"\n---\n\n# Prep: x\n')).toBe('# Prep: x\n');
    expect(packBody('# Prep: y\n')).toBe('# Prep: y\n');
  });

  it('opens a meeting recap from Recordings', () => {
    expect(recordingsSource).toContain('openMeetingRecap(');
  });
});
