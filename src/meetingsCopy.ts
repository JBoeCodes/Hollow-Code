/**
 * How meetings reach Hollow and its agents (P1-20): one wording for
 * Settings, Connectors' Recordings subhead, Library's Meetings and Kept
 * transcripts collections, Recordings' sign-in prompt and the agent brief
 * (`MEETINGS_BRIEF` in src-tauri/src/chat.rs repeats PLAUD_FOR_AGENTS and
 * CALENDAR_SOURCE; meetingsCopy.test.ts checks it). Plaud has two separate
 * sign-ins: Hollow's own, for Recordings, and the agents' connector.
 */
export const RECORDINGS_IN_HOLLOW = 'Recordings in Hollow uses this Plaud sign-in and reads Plaud on demand; nothing is copied unless you choose Keep a transcript copy.';
export const PLAUD_FOR_AGENTS = 'Plaud tools for agents come from the Plaud connector, with its own sign-in.';
export const CALENDAR_SOURCE = 'Calendar comes from the source you connect in Settings → Connectors.';
/** The whole paragraph: Settings, Connectors, Recordings. */
export const MEETINGS_ACCESS = `${RECORDINGS_IN_HOLLOW} ${PLAUD_FOR_AGENTS} ${CALENDAR_SOURCE}`;
/** Library's Kept transcripts (`recordings/`): what is here and where a kept
 *  copy goes instead (`recordings::keep_transcript` writes it into the
 *  linked workspace's sources/). */
export const KEPT_COPIES = 'Recordings stay in Plaud. Transcripts imported earlier are here; Keep a transcript copy saves one into the sources of the workspace you choose.';
/** The label of Settings' Plaud row, which Recordings' sign-in button opens. */
export const RECORDINGS_SIGN_IN_ROW = 'Recordings in Hollow';
