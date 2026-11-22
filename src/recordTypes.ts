/** Master plan F4: records (`records.rs`). */
export interface DecisionRecord {
  path: string;
  text: string;
  decidedAt: string;
  decidedBy: string;
  /** `global` or a workspace id. */
  scope: string;
  status: 'settled' | 'superseded' | 'revisit' | 'retracted';
  supersedes: string | null;
  supersededBy: string | null;
  evidence: string | null;
  source: string | null;
  /** DR1: options turned down, with why (empty on an older record). */
  rejected?: { option: string; reason: string | null }[];
  /** DR1: a hard constraint, set by hand in the record. */
  constraint?: boolean;
  derivesFrom?: string[];
  /** DR1b: the tracker issue key its build-log comment goes on, with its why and how it was checked. */
  issue?: string | null;
  why?: string | null;
  verified?: string | null;
}
