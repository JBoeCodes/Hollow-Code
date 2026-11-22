// B4: words for the Approvals row in Settings, Devices, Companion devices.
// Pure (no imports), so the node vitest suite can load it.

/** The waits a person can pick for a card while an iPhone can answer it. */
export const APPROVAL_WAIT_CHOICES = [
  { seconds: 120, label: '2 minutes' },
  { seconds: 300, label: '5 minutes' },
  { seconds: 600, label: '10 minutes' },
  { seconds: 900, label: '15 minutes' },
] as const;

/** One line for the row: off, or on with the fingerprint both screens show and when it was set up. */
export function approvalsLine(
  keys: { fingerprint: string; enrolledAt: string } | null,
  when: (iso: string) => string,
): string {
  if (!keys) return 'Approvals from this iPhone: off. A card waits for this Mac’s window.';
  return `Approvals from this iPhone: on · key ${keys.fingerprint} · since ${when(keys.enrolledAt)}`;
}
