// The divider before the first turn that arrived since the person last had this conversation open
// (chatVisitMark.ts). A thin rule with the count; Dismiss is the click path that clears it.
import { watermarkText } from '../../chatVisitMark';

export function ThreadVisitWatermark(props: { count: number; onDismiss: () => void }) {
  return (
    <div class="chat-visit-mark" role="separator" aria-label={watermarkText(props.count)}>
      <span class="chat-visit-mark-label">{watermarkText(props.count)}</span>
      <button type="button" class="chat-visit-mark-dismiss" onClick={() => props.onDismiss()} aria-label="Dismiss the new messages marker">Dismiss</button>
    </div>
  );
}
