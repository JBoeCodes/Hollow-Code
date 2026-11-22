/**
 * The last connector checks (next-level P16), shared by Settings,
 * Connectors and the composer's next-turn connector list. Settings runs the
 * checks; the composer only reads them, so opening a conversation never
 * probes anything. Kept in localStorage (`hollow.connectorChecks.v1`) so a
 * row can say "checked 2h ago as jo@x.com in Acme" after a restart; age and
 * login rules are applied by `connectorCapability`, never here.
 */
import { createSignal } from 'solid-js';
import type { ConnectorStatus, M365Status } from '../connectorsLibrary';
import type { M365Check } from '../connectorCapability';

const KEY = 'hollow.connectorChecks.v1';
interface Stored { statuses: Record<string, ConnectorStatus>; m365: M365Check | null }

function read(): Stored {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (v && typeof v === 'object') return { statuses: v.statuses && typeof v.statuses === 'object' ? v.statuses : {}, m365: v.m365 ?? null };
  } catch { /* no storage, or a bad value: start empty */ }
  return { statuses: {}, m365: null };
}
const initial = typeof localStorage === 'undefined' ? { statuses: {}, m365: null } : read();

export const [connectorChecks, setConnectorChecks] = createSignal<Record<string, ConnectorStatus>>(initial.statuses);
export const [m365Check, setM365Check] = createSignal<M365Check | null>(initial.m365);

function persist() {
  try { localStorage.setItem(KEY, JSON.stringify({ statuses: connectorChecks(), m365: m365Check() })); } catch { /* not fatal */ }
}

/** One finished claude.ai check: it replaces every earlier result (it covers
 *  every claude.ai connector in the library). Missing times are stamped now. */
export function recordConnectorChecks(list: ConnectorStatus[], at = new Date().toISOString()) {
  setConnectorChecks(Object.fromEntries(list.map(s => [s.id, { ...s, checkedAt: s.checkedAt ?? at }])));
  persist();
}

/** One Microsoft 365 sign-in check (`m365_status`, or a sign-in's answer). */
export function recordM365Check(s: M365Status, at = new Date().toISOString()) {
  setM365Check({ ...s, checkedAt: at });
  persist();
}
