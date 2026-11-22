/**
 * Canvas C3 wave 2 C4: how often agents answer with a canvas. Settings,
 * Chat → Canvases is `AgentSettings.canvases` (`off` or `most`; absent is the
 * default, When it helps); a chat's own choice from the model menu wins for
 * that chat's turns (`chat_send`'s `canvases`). The backend turns the choice
 * into the "Visual answers in Hollow" prompt line
 * (`src-tauri/src/visual_prompt.rs`). Pure, so vitest loads it in node.
 */
import type { CanvasFrequency } from './generated/ipc';

export type { CanvasFrequency };

export const CANVAS_FREQUENCIES: readonly CanvasFrequency[] = ['off', 'helps', 'most'];

export const CANVAS_FREQUENCY_LABELS: Readonly<Record<CanvasFrequency, string>> = {
  off: 'Off',
  helps: 'When it helps',
  most: 'Most replies',
};

/** A known choice, or null (absent, empty or unknown). */
export function parseCanvasFrequency(value: unknown): CanvasFrequency | null {
  return typeof value === 'string' && (CANVAS_FREQUENCIES as readonly string[]).includes(value)
    ? (value as CanvasFrequency) : null;
}

/** The saved setting: absent or unknown reads as the default. */
export function settingCanvases(settings: { canvases?: string | null } | null | undefined): CanvasFrequency {
  return parseCanvasFrequency(settings?.canvases) ?? 'helps';
}

/** What a chat's turns use: its own choice, else the setting. */
export function effectiveCanvases(chat: CanvasFrequency | null, setting: CanvasFrequency): CanvasFrequency {
  return chat ?? setting;
}

/** The settings save value: the default is never stored. */
export function canvasesForSave(choice: CanvasFrequency | null | undefined): CanvasFrequency | undefined {
  return choice && choice !== 'helps' ? choice : undefined;
}

/** Where a conversation's own choice is kept (localStorage), by its Hollow
 *  session key. */
export function chatCanvasesStorageKey(sessionKey: string): string {
  return `hollow.canvases.${sessionKey}`;
}

/**
 * The chat's choice after the conversation on screen changes from `prev` to
 * `next` (Hollow session keys). A new conversation getting its first key
 * keeps the choice made before its first send (and `persist` says to store
 * it under the new key); moving to another conversation, or to none, reads
 * that conversation's stored choice (`stored`), null when it made none.
 */
export function chatCanvasesOnKeyChange(
  prev: string | null,
  next: string | null,
  current: CanvasFrequency | null,
  stored: CanvasFrequency | null,
): { choice: CanvasFrequency | null; persist: boolean } {
  if (prev === next) return { choice: current, persist: false };
  if (!next) return { choice: null, persist: false };
  if (!prev && stored === null && current !== null) return { choice: current, persist: true };
  return { choice: stored, persist: false };
}

/** The model menu's row value: "Most replies", or "When it helps · setting"
 *  when the chat follows Settings. */
export function canvasesMenuValue(chat: CanvasFrequency | null, setting: CanvasFrequency): string {
  return chat ? CANVAS_FREQUENCY_LABELS[chat] : `${CANVAS_FREQUENCY_LABELS[setting]} · setting`;
}
