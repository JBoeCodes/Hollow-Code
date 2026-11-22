import { createSignal } from 'solid-js';

/** Shared invalidation token for the Today, People, and Project commitment
 * views. Durable state remains in commitments/*.md; this only coordinates
 * refetches after a modal/status action. */
export const [commitmentRevision, setCommitmentRevision] = createSignal(0);
export function refreshCommitments() { setCommitmentRevision(value => value + 1); }
