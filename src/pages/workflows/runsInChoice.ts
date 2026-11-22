// Run inside this chat (overhaul F1): where an automation's runs go. A new
// chat each run (no binding), or one conversation (`inChat`, a session key),
// where each run is a new turn. Pure helpers for the editor's Runs in choice
// and the chat menu's Schedule in this chat… request, so they cost the App
// chunk nothing.
import { createSignal } from 'solid-js';

/** The Runs in value for a new chat each run. */
export const NEW_CHAT = '';

export interface RunsInChat {
  key: string;
  title: string;
}

export interface RunsInOption {
  value: string;
  label: string;
}

/** The chat a Schedule in this chat… request names, read once by the
 *  automations page when it opens its new-automation form. */
export const [newAutomationChat, setNewAutomationChat] = createSignal<RunsInChat | null>(null);

/** The choices: a new chat each run, and the bound (or requested) chat. */
export function runsInOptions(chat: RunsInChat | null): RunsInOption[] {
  const options: RunsInOption[] = [{ value: NEW_CHAT, label: 'New chat each run' }];
  if (chat?.key) options.push({ value: chat.key, label: `This chat: ${chat.title.trim() || 'Untitled conversation'}` });
  return options;
}

/** One line under the choice saying what it means. */
export function runsInHint(value: string): string {
  return value === NEW_CHAT
    ? 'Each run starts a new chat with the agent, model and folder below.'
    : 'Each run is a new turn in this chat, by its own agent and model and with its tools, at Edits at most, so allowed and disallowed tools must be empty. Its usage counts toward this automation’s budget. It waits behind an open question or a running turn, and fails in the run log if the chat is archived or deleted.';
}

/** What a save sends: the choice when it changed, else nothing (keep it). */
export function inChatForSave(current: string, hydrated: string): string | undefined {
  return current === hydrated ? undefined : current;
}
