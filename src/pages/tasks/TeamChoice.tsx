// The team controls of the Tasks page, shown only while the mirror holds two
// teams (`trackerTeams.ts`): a filter in the toolbar (All teams, AI, HOL)
// and a team choice for New issue and Track in. Plain buttons with a visible
// pressed state and an accessible name; no keyboard shortcut.
import { For } from 'solid-js';
import { teamChoices, teamName } from '../../trackerTeams';

/** All teams / one team, as a pressed-button group (the view tabs' look). */
export function TeamFilter(props: { teamKeys: readonly string[]; value: string | null; onChange: (key: string | null) => void }) {
  return (
    <div class="tracker-views tracker-team-filter" role="group" aria-label="Team">
      <button type="button" class="tracker-view" data-team="all" aria-pressed={props.value === null} title="Issues from both teams"
        onClick={() => props.onChange(null)}>All teams</button>
      <For each={teamChoices(props.teamKeys)}>{key => (
        <button type="button" class="tracker-view" data-team={key} aria-pressed={props.value === key} title={teamName(key)}
          onClick={() => props.onChange(key)}>{key}</button>
      )}</For>
    </div>
  );
}

/** Which team a new issue or project goes to (AI by default). */
export function TeamPicker(props: { teamKeys: readonly string[]; value: string; onChange: (key: string) => void }) {
  return (
    <div class="tracker-views tracker-team-filter" role="group" aria-label="Create in team">
      <For each={teamChoices(props.teamKeys)}>{key => (
        <button type="button" class="tracker-view" data-team={key} aria-pressed={props.value === key} title={teamName(key)}
          onClick={() => props.onChange(key)}>{teamName(key)} ({key})</button>
      )}</For>
    </div>
  );
}
