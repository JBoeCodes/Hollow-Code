// The review card's IPC (CX20; Rust `context_review.rs` and the CX19 scan in
// `context_scan/`). Its own module (calls keep api.ts's mock routing), so
// the App chunk carries none of it: only the lazy `components/ContextReviewCard.tsx`
// imports it. Accept takes exactly one id: there is no batch form.
import { invoke } from './api';
import type { ContextReviewAccepted, ContextReviewLayer, ContextReviewList } from './generated/ipc';

export type { ContextReviewAccepted, ContextReviewChoice, ContextReviewFlag, ContextReviewItem, ContextReviewLayer, ContextReviewList } from './generated/ipc';

/** A scan just started (`context_scan_api::ScanStarted`). */
export interface ContextScanStarted { scanId: string; roots: string[] }

/** A scan's progress (`context_scan::ScanStatus`). */
export interface ContextScanStatus {
  scanId: string;
  state: 'scanning' | 'ready' | 'cancelled' | 'failed';
  found: number;
  error?: string;
}

/** Scan the default agent folders (the person's click). */
export const startContextScan = () => invoke<ContextScanStarted>('start_context_scan', {});

export const contextScanStatus = (scanId: string) => invoke<ContextScanStatus | null>('context_scan_status', { scanId });

/** The card's items: scan `scanId`'s (else the newest scan's) and the queued org facts. Reads only. */
export const contextReviewList = (scanId?: string | null) => invoke<ContextReviewList>('context_review_list', { scanId: scanId ?? null });

/** Accept ONE item into one layer. */
export const contextReviewAccept = (id: string, layer: ContextReviewLayer, scanId?: string | null) =>
  invoke<ContextReviewAccepted>('context_review_accept', { id, layer, scanId: scanId ?? null });

export const contextReviewSkip = (id: string) => invoke<boolean>('context_review_skip', { id });

export const contextReviewUnskip = (id: string) => invoke<boolean>('context_review_unskip', { id });
