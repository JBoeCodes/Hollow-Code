/** Master plan F3: the extraction pipeline (`extract.rs`). */
export type ExtractSource =
  | { kind: 'recording'; path: string }
  | { kind: 'chat'; sessionKey: string; fromTurn?: number; toTurn?: number };

export interface ExtractReport {
  sourceId: string;
  /** `skipped`: the same content was extracted before (no model call). */
  status: 'extracted' | 'skipped';
  queued: Array<{ kind: string; text: string; reviewKind: string; reviewId: string; flagged: boolean }>;
  dropped: Array<{ text: string; reason: string }>;
}

export interface ExtractStatus {
  sources: Array<{ sourceId: string; title: string; extractedAt: string; queued: number; dropped: number }>;
  pending: { tasks: number; commitments: number; records: number };
}
