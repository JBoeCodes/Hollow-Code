import { createUniqueId } from 'solid-js';
import { ChatMessage } from './ChatMessage';
import '../styles/proposed-plan.css';

export default function ProposedPlan(props: { text: string; disabled: boolean; onChoice: (choice: 'accept' | 'refine') => void }) {
  const title = createUniqueId();
  return <section class="chat-proposed-plan" aria-labelledby={title}>
    <h3 id={title}>Proposed plan</h3>
    <details><summary>Review the plan</summary><ChatMessage text={props.text} role="assistant" class="turn-md assistant" /></details>
    <p>Accept or refine prepares a message for you to review and send. Your permission mode stays unchanged.</p>
    <div class="chat-proposed-plan-actions">
      <button type="button" class="btn primary" disabled={props.disabled} onClick={() => props.onChoice('accept')}>Accept plan</button>
      <button type="button" class="btn ghost" disabled={props.disabled} onClick={() => props.onChoice('refine')}>Refine plan</button>
    </div>
  </section>;
}
