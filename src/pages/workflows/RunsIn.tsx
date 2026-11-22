import { Select } from '../../components/Select';
import { runsInHint, runsInOptions, type RunsInChat } from './runsInChoice';

/** The automation's Runs in choice (overhaul F1): a new chat each run, or
 *  one conversation where each run is a new turn. */
export function RunsIn(props: { value: string; chat: RunsInChat | null; onChange: (value: string) => void }) {
  return (
    <div class="workflow-field span-2">
      <span class="workflow-label">Runs in</span>
      <Select label="Runs in" value={props.value} onChange={props.onChange} options={runsInOptions(props.chat)} />
      <span class="workflow-hint">{runsInHint(props.value)}</span>
    </div>
  );
}
