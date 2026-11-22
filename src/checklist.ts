// Checklist dialogs (`showChecklist`): the row shape and how the dialog's
// answer (the ticked ids, as JSON) is read back. Pure, so it is testable
// without the store.

/** One row of a checklist dialog. */
export interface ChecklistRow { id: string; label: string; detail?: string; checked: boolean }

/** The ticked ids a checklist dialog sends back (JSON), kept to its rows
 *  and in their order; anything malformed reads as nothing ticked. */
export function parseTicked(value: string | undefined, rows: readonly ChecklistRow[]): string[] {
  let ids: unknown;
  try { ids = JSON.parse(value ?? '[]'); } catch { return []; }
  if (!Array.isArray(ids)) return [];
  return rows.map(r => r.id).filter(id => ids.includes(id));
}
