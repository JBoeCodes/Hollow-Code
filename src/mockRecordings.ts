/** Browser-mock Plaud recordings (next-level P11): one with transcript and
 *  summary, one with a summary only, one with a transcript only, and one with
 *  neither. `mockPlaudOff` → not connected; `mockRecordingLinksDamaged` → the
 *  listing fails as for a damaged links file. Links in `mockRecordingLinks`. */
const day = (n: number) => { const d = new Date(); d.setDate(d.getDate() - n); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };
const RECS = () => [
  { id: 'rec0a1b2c3d4e5f6', name: 'Weekly ops sync', date: day(0), duration: '42m10s' },
  { id: 'rec1f00ba4c0ffee', name: 'Vendor security call', date: day(1), duration: '28m02s' },
  { id: 'rec2a7a7e5ca1e00', name: 'Site visit: Garage 14', date: day(3), duration: '15m40s' },
  { id: 'rec3bead0000beef', name: 'Board prep (no notes)', date: day(6), duration: '6m12s' },
];
const DETAIL: Record<string, { transcript: { start: string; end: string; speaker: string | null; text: string }[] | null; summary: string | null; audio: string }> = {
  rec0a1b2c3d4e5f6: {
    transcript: [
      { start: '00:05', end: '00:18', speaker: 'Dana', text: 'Mid-Atlantic overtime is down to within one percent of plan for the last two weeks.' },
      { start: '01:02', end: '01:20', speaker: 'Sam', text: 'I will confirm the float supervisor budget with finance before Friday.' },
      { start: '02:40', end: '02:55', speaker: 'Dana', text: 'Can someone draft the Q4 hiring plan so we can review it next week?' },
      { start: '04:10', end: '04:25', speaker: null, text: 'Decision: we keep two permanent float supervisors in the Mid-Atlantic.' },
    ],
    summary: '## Overview\nOvertime is back within plan; staffing decisions for Q4.\n\n## Decisions\n- Keep two permanent float supervisors in the Mid-Atlantic\n\n## Action items\n- Sam: Confirm the float supervisor budget with finance\n- Draft the Q4 hiring plan for review next week',
    audio: 'available',
  },
  rec1f00ba4c0ffee: { transcript: null, summary: '## Action items\n- Send the security questionnaire to both vendors\n- Collect their SOC 2 reports (owner: Priya)', audio: 'unavailable' },
  rec2a7a7e5ca1e00: { transcript: [{ start: '00:10', end: '00:30', speaker: null, text: 'The gate arm on level two sticks after rain.' }], summary: null, audio: 'available' },
  rec3bead0000beef: { transcript: null, summary: null, audio: 'unavailable' },
};
const links = () => JSON.parse(localStorage.getItem('mockRecordingLinks') ?? '{}');
export function recordingsFixture(command: string, args: Record<string, unknown> | undefined): { value: unknown } | null {
  if (command === 'list_recordings') {
    const fetchedAt = new Date().toISOString();
    // `mockRecordingLinksDamaged`: the backend's error for a damaged links file (B·F08).
    if (localStorage.getItem('mockRecordingLinksDamaged')) throw 'Recording link store was damaged (expected value at line 1 column 1). It was moved to /mock/vault/.hollow/recording-links.json.corrupt-20261002T120000.000Z; a fresh one starts now.';
    if (localStorage.getItem('mockPlaudOff')) return { value: { state: 'notConnected', fetchedAt, source: 'Plaud · read only', recordings: [], unparsed: 0, error: null, links: links() } };
    return { value: { state: 'ok', fetchedAt, source: 'Plaud · read only', recordings: RECS(), unparsed: 0, error: null, links: links() } };
  }
  if (command === 'read_recording') {
    const id = String(args?.id); const d = DETAIL[id]; const r = RECS().find(x => x.id === id);
    if (!d || !r) throw `not a Plaud recording id: ${id}`;
    return { value: { id, name: r.name, fetchedAt: new Date().toISOString(), transcript: d.transcript, transcriptState: d.transcript ? 'available' : 'unavailable', summary: d.summary, summaryState: d.summary ? 'available' : 'unavailable', audio: d.audio, link: links()[id] ?? null, errors: [] } };
  }
  if (command === 'recording_audio_url') { const d = DETAIL[String(args?.id)]; if (d?.audio !== 'available') throw 'Audio isn’t available for this recording.'; return { value: `https://plaud.example/audio/${String(args?.id)}?expires=24h` }; }
  if (command === 'set_recording_link') {
    const all = links(); const id = String(args?.id);
    if (!args?.workspaceId && !args?.eventId) { delete all[id]; localStorage.setItem('mockRecordingLinks', JSON.stringify(all)); return { value: null }; }
    all[id] = { workspaceId: args?.workspaceId ?? null, eventId: args?.eventId ?? null, eventTitle: args?.eventTitle ?? null, name: args?.name ?? null, linkedAt: new Date().toISOString(), keptCopy: all[id]?.keptCopy ?? null };
    localStorage.setItem('mockRecordingLinks', JSON.stringify(all)); return { value: all[id] };
  }
  return null;
}
