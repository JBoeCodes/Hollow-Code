// "Turn on search by meaning" (Settings plan PR 2, section 5.2 item 6): the
// link Library's search shows while semantic search is off, opening Settings
// on the switch that turns it on. Self-contained so it survives Library's
// layout moving: place it anywhere beside a search box.
import { Show } from 'solid-js';
import { openSettingsFromOutside, semanticSearchEnabled } from '../store';

/** The switch the link lands on: System, Advanced, Search & indexes. */
export const SEMANTIC_SEARCH_TARGET = { section: 'system', control: 'Index my vault for semantic search' } as const;

/** Open Settings on the switch, with focus (the jump focuses the row's
 *  control). One guarded jump: an unsaved draft's refusal leaves no request. */
export function openSemanticSearchSettings(
  deps: { open: (section: string, control: string) => unknown } = { open: openSettingsFromOutside },
) {
  deps.open(SEMANTIC_SEARCH_TARGET.section, SEMANTIC_SEARCH_TARGET.control);
}

export function SemanticSearchLink() {
  return <Show when={!semanticSearchEnabled()}>
    <button type="button" class="btn-link semantic-search-link" title="Find notes by what they mean: an on-device index, off by default" onClick={() => openSemanticSearchSettings()}>Turn on search by meaning</button>
  </Show>;
}
