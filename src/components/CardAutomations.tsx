import { For, Show, createMemo } from 'solid-js';
import { appendToChatDraft, chatAgentSessionKey } from '../store/chatRuntime';
import { openWorkflowBySlug, workflowsList } from '../store/runtime';
import { ADD_AUTOMATION_PROMPT, automationWords, chatAutomations } from '../cardAutomationsModel';

/** The card's Automations body (parity 5.5, D22): the automations that run in
 *  this chat, each opening its detail on the Automations page, and a button
 *  that drafts a request to add one. Reads the same list as that page. */
export function CardAutomations() {
  const mine = createMemo(() => chatAutomations(workflowsList(), chatAgentSessionKey()));
  return (
    <div class="side-card-automations">
      <Show when={mine().length} fallback={<p class="side-card-empty-note">None run in this chat yet.</p>}>
        <ul class="side-card-files">
          <For each={mine()}>{w => (
            <li><button type="button" class="side-card-file" title={`Open ${w.title || w.slug} on the Automations page`} onClick={() => openWorkflowBySlug(w.slug)}>
              <span>{w.title || w.slug}</span><span class="file-chip-folder">{automationWords(w)}</span>
            </button></li>
          )}</For>
        </ul>
      </Show>
      <button type="button" class="side-card-empty" onClick={() => appendToChatDraft(ADD_AUTOMATION_PROMPT)}>Add an automation</button>
    </div>
  );
}
