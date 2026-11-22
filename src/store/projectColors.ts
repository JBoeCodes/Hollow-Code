import { createSignal } from 'solid-js';

/** Project colors the person picked (t3code parity 4.4, A8): project id to color
 *  id, kept on this Mac only. A project with no entry uses its default color
 *  (`projectIconColors.ts`). */
const KEY = 'hollow.projectColors';

function read(): Record<string, string> {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}');
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return {};
    return Object.fromEntries(Object.entries(raw).filter((e): e is [string, string] => typeof e[1] === 'string'));
  } catch { return {}; }
}

export const [projectColorPicks, setProjectColorPicks] = createSignal<Record<string, string>>(read());

/** Pick a color for a project, or `null` to go back to its default. */
export function pickProjectColor(id: string, color: string | null) {
  const next = { ...projectColorPicks() };
  if (color) next[id] = color; else delete next[id];
  setProjectColorPicks(next);
  try { localStorage.setItem(KEY, JSON.stringify(next)); } catch { /* storage full or off: the pick lasts until restart */ }
}
