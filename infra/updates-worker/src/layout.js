// The R2 layout of the `hollow-updates` bucket and the names that may appear
// in it (updates design section 4). Pure: no I/O. The Worker, the release
// builder (U2) and promotion (U8) all build keys from these helpers, so a key
// is never assembled from request text.

/** The channels a person can be on. Order matters: the install page offers
 *  the first one a person is allowed. */
export const CHANNELS = Object.freeze(['main', 'nightly']);

/** A release version: `0.6.<n>` where n is the first-parent commit count
 *  (design section 2). No leading zeros, at most nine digits. */
export const VERSION_RE = /^0\.6\.(?:0|[1-9][0-9]{0,8})$/;

/** The updater archive, its minisign signature, the notarized disk image and
 *  the release notes, in upload order (artifacts first, notes last). */
export const BUILD_FILES = Object.freeze(['Hollow.app.tar.gz', 'Hollow.app.tar.gz.sig', 'Hollow.dmg', 'notes.json']);

/** What a channel manifest needs in the bucket before it may be served. The
 *  DMG is not on this list: the updater never reads it, the install page
 *  checks it on its own. */
export const MANIFEST_PREREQUISITES = Object.freeze(['Hollow.app.tar.gz', 'Hollow.app.tar.gz.sig', 'notes.json']);

/** The platform key of the Tauri static updater manifest. Apple silicon only
 *  (design section 13, default 10). */
export const PLATFORM = 'darwin-aarch64';

/** @param {unknown} name @returns {name is 'main' | 'nightly'} */
export function isChannel(name) {
  return typeof name === 'string' && CHANNELS.includes(name);
}

/** @param {unknown} version @returns {version is string} */
export function isVersion(version) {
  return typeof version === 'string' && VERSION_RE.test(version);
}

/** @param {unknown} file @returns {file is string} */
export function isBuildFile(file) {
  return typeof file === 'string' && BUILD_FILES.includes(file);
}

/** The build's own copy of the Tauri manifest it was first published with
 *  (`builds/<v>/manifest.json`, row U8): promotion to Main copies it, so any
 *  kept Nightly can be published, not only the newest. Stored, never served:
 *  it is not in BUILD_FILES, so the Worker's build route 404s it. */
export const BUILD_RECORD = 'manifest.json';

/** The R2 key of a build's stored manifest. Throws on a bad version.
 *  @param {string} version */
export function buildRecordKey(version) {
  if (!isVersion(version)) throw new Error('bad version');
  return `builds/${version}/${BUILD_RECORD}`;
}

/** The R2 key of a channel manifest. Throws on a name outside CHANNELS.
 *  @param {string} name */
export function channelKey(name) {
  if (!isChannel(name)) throw new Error('unknown channel');
  return `channels/${name}.json`;
}

/** The R2 key of one build file. Throws on a bad version or file name.
 *  @param {string} version @param {string} file */
export function buildKey(version, file) {
  if (!isVersion(version)) throw new Error('bad version');
  if (!isBuildFile(file)) throw new Error('bad build file');
  return `builds/${version}/${file}`;
}

/** The path the Worker serves a build file at (same shape as the key).
 *  @param {string} version @param {string} file */
export function buildPath(version, file) {
  return `/${buildKey(version, file)}`;
}

/** The R2 key of the people list. Never served. */
export const PEOPLE_KEY = 'people.json';
