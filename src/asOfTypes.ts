/** Master plan F5/#17: `history_read_as_of` (`asof.rs`). */
export interface AsOfResult {
  path: string;
  asOf: string;
  /** `snapshot`, `history`, `absent` or `not-captured`. */
  source: 'snapshot' | 'history' | 'absent' | 'not-captured';
  capturedAt: string | null;
  runId: string | null;
  text: string | null;
  note: string;
}
