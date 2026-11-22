// The page a teammate opens from their install link (design section 11):
// a link to the notarized DMG and a "Connect Hollow" button that hands the
// token to the app through `hollow://updates/connect?token=…`. Only built
// after the token checked out; every value placed in the HTML is escaped.

/** @param {string} value */
export function escapeHtml(value) {
  return String(value).replace(/[&<>"'`=]/g, ch => `&#${ch.charCodeAt(0)};`);
}

/**
 * @param {{ token: string, name: string, version: string | null, dmgPath: string | null }} page
 *   `dmgPath` is the `/builds/<v>/Hollow.dmg` path, or null when no complete
 *   build with a DMG is published on the person's channel yet.
 * @returns {string}
 */
export function installPage({ token, name, version, dmgPath }) {
  const q = encodeURIComponent(token);
  const connect = `hollow://updates/connect?token=${q}`;
  const download = dmgPath && version
    ? `<p><a class="button" href="${escapeHtml(`${dmgPath}?token=${q}`)}" download="Hollow.dmg">Download Hollow ${escapeHtml(version)}</a></p>
<p>Open the disk image and drag Hollow to Applications, then open Hollow.</p>`
    : '<p>No build is published for you yet. Try this link again later.</p>';
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<meta name="referrer" content="no-referrer">
<title>Install Hollow</title>
<style>
body { font: 16px/1.5 -apple-system, BlinkMacSystemFont, sans-serif; max-width: 34rem; margin: 4rem auto; padding: 0 1.5rem; color: #1d1d1f; background: #fbfbfa; }
h1 { font-size: 1.6rem; margin-bottom: 0.25rem; }
.button { display: inline-block; padding: 0.6rem 1.1rem; border-radius: 8px; background: #1d1d1f; color: #fff; text-decoration: none; }
.note { color: #6e6e73; font-size: 0.9rem; }
</style>
</head>
<body>
<h1>Hollow for ${escapeHtml(name)}</h1>
<h2>1. Install</h2>
${download}
<h2>2. Connect</h2>
<p>With Hollow open, connect it so it can get updates.</p>
<p><a class="button" href="${escapeHtml(connect)}">Connect Hollow</a></p>
<p class="note">This link is yours alone. Don't share it: anyone with it can download Hollow until it is revoked.</p>
</body>
</html>
`;
}
