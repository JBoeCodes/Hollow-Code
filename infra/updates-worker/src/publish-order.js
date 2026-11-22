// The upload order for a release (design section 4: "manifests are written
// last"). Pure, so the release builder (U2) and promotion to Main (U8) import
// the same contract the Worker enforces: a channel manifest is served only
// when every file in MANIFEST_PREREQUISITES exists, so an uploader that
// follows this order never exposes a half-uploaded build, and one that stops
// part way leaves the channel on its previous build.

import { BUILD_FILES, MANIFEST_PREREQUISITES, buildKey, buildRecordKey, channelKey, isChannel, isVersion } from './layout.js';

/**
 * @typedef {{ key: string, kind: 'artifact' | 'record' | 'notes' | 'manifest', channel?: string }} UploadStep
 */

/**
 * Every R2 write for publishing `version` to `channels`, in the order they
 * must happen: the archive, its signature and the DMG, then the build's own
 * manifest record (`builds/<v>/manifest.json`, kind `record`, never served;
 * promotion to Main copies it), then `notes.json`, then each channel
 * manifest. Upload one step at a time and stop at the first
 * failure. Throws on a bad version, an unknown channel, a repeated channel or
 * an empty channel list.
 *
 * @param {{ version: string, channels: string[] }} release
 * @returns {UploadStep[]}
 */
export function publishOrder({ version, channels }) {
  if (!isVersion(version)) throw new Error('bad version');
  if (!Array.isArray(channels) || channels.length === 0) throw new Error('no channel');
  const seen = new Set();
  for (const channel of channels) {
    if (!isChannel(channel)) throw new Error('unknown channel');
    if (seen.has(channel)) throw new Error('repeated channel');
    seen.add(channel);
  }
  /** @type {UploadStep[]} */
  const steps = BUILD_FILES.filter(file => file !== 'notes.json').map(file => ({ key: buildKey(version, file), kind: 'artifact' }));
  steps.push({ key: buildRecordKey(version), kind: 'record' });
  steps.push({ key: buildKey(version, 'notes.json'), kind: 'notes' });
  for (const channel of channels) steps.push({ key: channelKey(channel), kind: 'manifest', channel });
  return steps;
}

/**
 * What must already be in the bucket before `channels/<name>.json` may point
 * at `version`. Promotion (U8) writes no build files, so it checks these keys
 * exist (and reads the build's record, `buildRecordKey`) and then writes the
 * one manifest.
 *
 * @param {string} version
 * @returns {string[]}
 */
export function manifestPrerequisites(version) {
  return MANIFEST_PREREQUISITES.map(file => buildKey(version, file));
}
