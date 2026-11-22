import { createUniqueId } from 'solid-js';
import { Select } from '../../components/Select';
import { ownerMacs } from './ownerMacs';
import { NO_OWNER, RUNS_ON_APPROVAL, runsOnHint, runsOnOptions, runsOnValue } from './runsOnChoice';

/** The automation's Runs on choice (M4 follow-up): no owner, this Mac, or a
 *  paired Mac. Its hint (one live region whose text changes) says what the
 *  choice means; a new owner says it is approved like any permission change.
 *  Both describe the select. */
export function RunsOn(props: { value: string; saved: string; shared: boolean; onChange: (value: string) => void }) {
  const id = createUniqueId();
  const hint = () => runsOnHint(props.value, ownerMacs(), props.shared);
  const changed = () => props.value !== NO_OWNER && props.value !== runsOnValue(props.saved, ownerMacs());
  return (
    <div class="workflow-field span-2 workflow-runs-on">
      <span class="workflow-label">Runs on</span>
      <Select label="Runs on" value={props.value} onChange={props.onChange} options={runsOnOptions(props.saved, ownerMacs())}
        describedBy={`runs-on-hint-${id} runs-on-approval-${id}`} />
      <span id={`runs-on-hint-${id}`} class="workflow-hint" classList={{ 'workflow-next-warn': hint().warn }} role="status">{hint().text}</span>
      <span id={`runs-on-approval-${id}`} class="workflow-hint">{changed() ? RUNS_ON_APPROVAL : ''}</span>
    </div>
  );
}
