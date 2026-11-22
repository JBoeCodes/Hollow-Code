/**
 * A steer the agent didn't take (P1-30): send it as the next message, or put
 * it back in the box. The chat's steer row; the caller hides it once sent,
 * so it goes once.
 */
export function SteerActions(props: { onSend: () => void; onPutBack: () => void }) {
  return (
    <div class="chat-steer-actions">
      <button type="button" class="btn small" onClick={() => props.onSend()}>Send as next message</button>
      <button type="button" class="btn small ghost" onClick={() => props.onPutBack()}>Put back in the box</button>
    </div>
  );
}
