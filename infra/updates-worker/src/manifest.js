// Channel manifests (design sections 2, 4 and 9). A manifest is Tauri's
// static updater JSON plus Hollow's own fields; the full format is in the
// README. The Worker serves one only when it parses, names the channel it was
// read from, points at its own version's archive, and every build file it
// needs is in the bucket with the signature it claims. Anything else is
// treated as an empty channel, so a half-uploaded or hand-broken publish is
// never offered.

import { MANIFEST_PREREQUISITES, PLATFORM, buildKey, buildPath, channelKey, isVersion } from './layout.js';

const COMMIT_RE = /^[0-9a-f]{40}$/;
const MAX_MANIFEST_BYTES = 256 * 1024;

/**
 * @typedef {{
 *   version: string, notes?: string, pub_date?: string, channel: string,
 *   commit?: string, buildId?: string, schema?: Record<string, number>,
 *   platforms: Record<string, { signature: string, url: string }>,
 * }} Manifest
 */

/**
 * Parses and checks a manifest read from `channels/<channel>.json`. Returns
 * null when it is not one the Worker may serve.
 * @param {string} text
 * @param {string} channel
 * @returns {Manifest | null}
 */
export function parseManifest(text, channel) {
  if (text.length > MAX_MANIFEST_BYTES) return null;
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    return null;
  }
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
  if (!isVersion(data.version)) return null;
  if (data.channel !== channel) return null;
  if (data.notes !== undefined && typeof data.notes !== 'string') return null;
  if (data.pub_date !== undefined && (typeof data.pub_date !== 'string' || Number.isNaN(Date.parse(data.pub_date)))) return null;
  if (data.commit !== undefined && (typeof data.commit !== 'string' || !COMMIT_RE.test(data.commit))) return null;
  if (data.buildId !== undefined && (typeof data.buildId !== 'string' || data.buildId.length === 0)) return null;
  if (data.schema !== undefined) {
    if (!data.schema || typeof data.schema !== 'object' || Array.isArray(data.schema)) return null;
    for (const [name, value] of Object.entries(data.schema)) {
      if (!/^[A-Za-z0-9_-]{1,64}$/.test(name) || !Number.isSafeInteger(value) || value < 0) return null;
    }
  }
  const platforms = data.platforms;
  if (!platforms || typeof platforms !== 'object' || Array.isArray(platforms)) return null;
  const entry = platforms[PLATFORM];
  if (!entry || typeof entry !== 'object') return null;
  if (typeof entry.signature !== 'string' || entry.signature.trim().length === 0) return null;
  if (typeof entry.url !== 'string' || !pointsAtOwnArchive(entry.url, data.version)) return null;
  return /** @type {Manifest} */ (data);
}

/**
 * True when `url` (absolute, or a path) names `/builds/<version>/Hollow.app.tar.gz`
 * with no query or fragment. The host is not trusted: the Worker rewrites it.
 * @param {string} url @param {string} version
 */
export function pointsAtOwnArchive(url, version) {
  let parsed;
  try {
    parsed = new URL(url, 'https://placeholder.invalid');
  } catch {
    return false;
  }
  if (parsed.protocol !== 'https:' || parsed.search !== '' || parsed.hash !== '') return false;
  return parsed.pathname === buildPath(version, 'Hollow.app.tar.gz');
}

/**
 * The manifest as served, rebuilt from the checked fields only: `platforms`
 * reduced to darwin-aarch64, its url rewritten to this Worker's own
 * `/builds/...` route on `origin`.
 * @param {Manifest} manifest @param {string} origin
 * @returns {Manifest}
 */
export function servedManifest(manifest, origin) {
  const entry = manifest.platforms[PLATFORM];
  /** @type {Manifest} */
  const served = {
    version: manifest.version,
    channel: manifest.channel,
    platforms: { [PLATFORM]: { signature: entry.signature, url: `${origin}${buildPath(manifest.version, 'Hollow.app.tar.gz')}` } },
  };
  // Only fields parseManifest checked; anything else in the stored JSON is dropped.
  if (manifest.notes !== undefined) served.notes = manifest.notes;
  if (manifest.pub_date !== undefined) served.pub_date = manifest.pub_date;
  if (manifest.commit !== undefined) served.commit = manifest.commit;
  if (manifest.buildId !== undefined) served.buildId = manifest.buildId;
  if (manifest.schema !== undefined) served.schema = { ...manifest.schema };
  return served;
}

/**
 * The channel's manifest if it is complete in the bucket, else null. Checks
 * every MANIFEST_PREREQUISITES file exists and is not empty, and that the
 * stored `.sig` is the signature the manifest carries.
 * @param {{ get(key: string): Promise<any>, head(key: string): Promise<any> }} bucket
 * @param {string} channel
 * @returns {Promise<Manifest | null>}
 */
export async function completeManifest(bucket, channel) {
  const object = await bucket.get(channelKey(channel));
  if (!object) return null;
  const manifest = parseManifest(await object.text(), channel);
  if (!manifest) return null;
  for (const file of MANIFEST_PREREQUISITES) {
    if (file === 'Hollow.app.tar.gz.sig') continue;
    const head = await bucket.head(buildKey(manifest.version, file));
    if (!head || !(head.size > 0)) return null;
  }
  const sig = await bucket.get(buildKey(manifest.version, 'Hollow.app.tar.gz.sig'));
  if (!sig) return null;
  if ((await sig.text()).trim() !== manifest.platforms[PLATFORM].signature.trim()) return null;
  return manifest;
}
