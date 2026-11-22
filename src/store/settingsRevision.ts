/**
 * Bumped when an agent changed a backend setting that a Settings panel holds
 * in its own read (overnight H6, 2026-10-15): Learning suggestions, the
 * extraction model and Retitle new chats. `appBridge.ts` bumps it on an
 * `app-changed` whose plan says `settings` (`planForChanged`); each of those
 * panels keys its resource on it, so it reads the setting again while open
 * instead of showing the value from when it opened. Its own module, with no
 * imports beyond Solid, so a Settings panel can read it without pulling the
 * bridge into its chunk.
 */
import { createSignal } from 'solid-js';

export const [settingsRevision, setSettingsRevision] = createSignal(0);

export function bumpSettingsRevision() {
  setSettingsRevision(n => n + 1);
}
