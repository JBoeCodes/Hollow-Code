# Updates

Hollow's update channels (Nightly and Main), the Update button, local branch
builds, teammate access and going back: the one guide to how Hollow updates.
Design: the 2026-11-14 updates design in the Hollow workspace (sections are
cited below). Rows U6 (going back), U7 (overnight), U8 (publishing to Main)
and U9 (people) fill in their sections below.

## How Hollow updates

**One place to update: the top bar's Update button** (design section 7). It
shows only when something is ready to install: a channel build downloaded and
checked, or a **Local build of <branch>** from a build workspace. Its popover
names the version and channel (or the branch and commit) and what's new:

- **Restart to update** when nothing runs. It uses the restart checks, so
  work that starts meanwhile stops it with a note instead of being cut off.
- **Update when chats finish** while a chat or automation runs: it waits with
  **no expiry** and can be cancelled from the button (the old 30-minute
  waiter is gone).
- **Not now** hides that version (or that Local build) until the next one.

After the restart the button's notice says "Updated to <version>" (or "to the
local build of <branch>"), "went back", or why nothing was installed.

**Channels.** Nightly gets every merged change that passes its checks; Main
only what Alex publishes. Settings, Updates picks one (and holds the
updates token, Check now and Install updates overnight, which installs a
ready channel update between 2 and 5 AM when the Mac is quiet and on power).

**The installer** is the app's own `hollow install-update`, run from the
new bundle while Hollow quits: one installer lock, a journal per install, the
new app verified, the old one kept as `/Applications/.Hollow.app.previous`,
the reopen check, and an automatic go back when the new app fails to start
twice. Flags, exit codes and journals: `docs/reference/build-and-ship.md`,
"In-app installer".

**Agents** never install a channel update. An agent may propose installing a
branch build with `build.install_built_app`, which always asks on a card
(Bypass included) and is refused for automations and unattended (overnight)
runs; the allowed build then waits in the same Update button.

## Local builds (U10)

A build workspace's **Changes, Hollow app, Build app** still builds the
worktree for testing (`self_install.rs`, one build job app-wide). When it
finishes, the build is offered in the Update button as **Local build of
<branch>** (`update_client/local.rs`, `Client::offer_local`): heading, commit,
the bundle's version, and what it brings (`install_brings`, the pull requests
and direct commits between the installed commit and the build, as plain
text). It installs through the same Restart to update and Update when chats
finish, by the same `hollow install-update`. The build workspace's Install and
restart card, its banner with Cancel, the agent install's 30-minute waiter
(`self_install/scheduled.rs`, `INSTALL_WAIT`, `scheduled-install-event`,
`pending_self_install`, `cancel_scheduled_install`) and the `install_built_app`
command and its helper script are gone.

- **A copy of its own:** when offered, the built `Hollow.app` is copied (an
  APFS clone, `ditto`) to `updates/local/<buildId>/Hollow.app`, and that copy
  is hashed, checked and passed as `--staged`, so a rebuild writing the cargo
  target can't change the bytes between the hash and the install. Other
  copies are removed when another build is offered or the build leaves the
  button, never while an install starts.
- **The bytes are the receipt's.** The copy is hashed against the build
  receipt (`self_install::bundle_digest`) when it is offered and again before
  the installer starts; a changed bundle is refused ("The built app changed
  after its build finished").
- **Arguments:** `--expected-version <the bundle's CFBundleShortVersionString>
  --expected-team <team> --allow-downgrade` (a branch build may be lower than
  an installed Nightly), plus the usual app, journal, snapshot and install id.
- **The team rule:** local builds are signed on this Mac (the Developer ID,
  or "Hollow Dev Signing" until U0) and not notarized. The expected team is
  the bundle's own TeamIdentifier only when it equals the configured team
  (TEAM000000, or `HOLLOW_UPDATE_TEAM` in debug builds), which then must pass
  the Developer ID requirement too; any other bundle is checked as ad hoc
  (`-`), so a self-signed build with no team passes and a build signed by
  another team is refused.
- **While it waits,** channel checks pause. A channel update that was ready
  is kept aside (`Inner.shelved`) and is back in the button, without a new
  download, as soon as the Local build leaves it (same channel, a downgrade
  still in its 24 hours); a channel switch or a disconnect drops it and
  leaves the Local build in place. No updates token is needed.
- **Update when chats finish is bound to its build:** the wait remembers
  whether it was armed for the channel (a newer channel build from a check
  keeps it) or for one Local build. Offering another Local build, or a
  Local build over a channel update, ends the wait, and the wait's own looks
  refuse any other build, so it never installs a build the person didn't
  choose. The agent's approved install offers its build, then arms its own
  wait.
- **"Updated" means the build runs:** a branch build can carry the running
  version, so after the restart a Local install reads as updated only when
  the running app's source fingerprint (or, without one, its clean commit)
  is the build's (`local::is_running`); else it is "not installed" and the
  build receipt says failed.
- **A new app build is refused** while a Local build's install is starting
  (`self_install::LOCAL_INSTALLING`).
- **It leaves the button** on Not now (remembered as
  `dismissedLocalBuild`, so the next launch doesn't offer it again), when a
  new build starts, or when it fails its checks at install. **Changes, Hollow
  app, Show in Update button** (`update_offer_local_build {id}`) offers it
  again. A finished build saved across a restart (`finished-build.json`) is
  offered again at launch unless it was hidden.
- **An agent's install** (`build.install_built_app`, always asks, now a verb
  with no Tauri command of its own): after Allow it checks the approved
  receipt, offers that build and starts its Update when chats finish, so it
  installs once the calling turn has ended and nothing else runs.
- **The record:** `last-update.json` carries the build's receipt and branch;
  after the restart the notice says "Updated to the local build of
  <branch>", and the build receipt (`last-result.json`, read by
  `self_install_receipt` and the Hollow repository lookup) is written once
  from the installer's outcome, already acknowledged, so the older
  install notice (`SelfInstallNotice`, which still shows an unseen record
  from an older Hollow) doesn't repeat it.
- **A review copy** (candidate build) refuses Local builds.

Types: `UpdateRelease.local` (`UpdateLocalBuild {workspaceId, branch,
buildId, dirty}`, null for a channel build), `UpdateLastResult.updated.local`,
`UpdateSettings.dismissedLocalBuild`.

Going back, publishing to Main and People have their own sections below:
[Publish to Main (U8)](#publish-to-main-u8), [People (U9)](#people-u9) and
[Go back (U6)](#go-back-u6).

## The update host (U3)

A Cloudflare Worker in [`infra/updates-worker/`](../infra/updates-worker/README.md)
in front of the R2 bucket `hollow-updates`, meant for
`updates.example.com` (deployed by U11, not yet). The README there is the
full contract; in short:

- `GET /channels/<main|nightly>.json`: the channel's Tauri updater manifest,
  with `Authorization: Bearer <token>`. 401 for a missing, malformed, unknown
  or revoked token (one identical answer), 403 for a channel the person may
  not use, 404 for an empty channel or one whose build is not completely
  uploaded.
- `GET /builds/<0.6.n>/<Hollow.app.tar.gz|Hollow.app.tar.gz.sig|Hollow.dmg|notes.json>`:
  the build files, same token; the DMG alone also takes `?token=` so the
  install page's link works in a browser. Single byte ranges are served.
- `GET /install/<token>`: the page a teammate opens: the DMG and a
  **Connect Hollow** button (`hollow://updates/connect?token=…`).
- `GET /me` (U9): `{name, channels}` for the bearer token, `no-store`; the
  same 401 as the other routes.

**Tokens:** 32 random bytes shown once as 43 base64url characters; the bucket
keeps the lowercase hex SHA-256 in `people.json` with the person's name and
allowed channels. A revoke reaches the Worker within 60 seconds (its
`people.json` cache).

**Manifests:** Tauri's static JSON (`version`, `notes`, `pub_date`,
`platforms["darwin-aarch64"]{signature,url}`) plus `channel`, `commit`,
`buildId` and `schema` (each store's `PRAGMA user_version`, read by Go back).

**Publish order:** archive, signature and DMG, then `notes.json`, then the
channel manifest last (`publishOrder` in
`infra/updates-worker/src/publish-order.js`, shared with the release builder
and promotion to Main).

## The release builder (U2)

`scripts/release/nightly.mjs` (design section 5) turns each new tip of
`origin/main` into a Nightly. It is meant to run on the Mac mini as
`runner`, every 5 minutes, from its own clone of the repository.

**One run:**

1. Take the run lock (`<state>/nightly.lock`: pid, start time, host). Another
   run holds it: exit quietly. It is taken over only when the holder is
   provably gone: same host and its pid is dead or now belongs to a process
   with another start time than the lock's `pidStart` (`ps -o lstart=`; a
   pid reused after a reboot), or the file is unreadable and older than
   `HOLLOW_NIGHTLY_LOCK_STALE_MINUTES`. A lock that can't be written is a
   skipped run with a log line. A live pid is never
   taken over, however old (the log says so; stopping a hung builder is a
   person's call). The takeover renames the lock aside and checks it moved
   the file it judged stale; if another runner replaced it meanwhile, that
   lock is put back and this run backs off.
2. **fetch:** `git fetch --no-tags --no-prune origin +refs/heads/main:refs/remotes/origin/main`
   and take the tip. Exit quietly when the tip is the last published Nightly,
   or the last failed one: a failed tip is never retried on its own; it waits
   for the next merge, or `--retry`. A failed fetch is logged only (no commit
   to name; a network blip notifies nobody).
3. **config**, before anything runs: `resolveSigning` must give a Developer
   ID identity with a team (not ad hoc, not the self-signed one) and
   notarization (all three `APPLE_API_*`); the updater key and the R2
   settings must be set. Otherwise nothing is built or published.
   Then the live `channels/nightly.json` is read: when it already names the
   tip (a run that died after writing the manifest but before `state.json`),
   the run records it and exits. Only then is a tip that already failed
   skipped.
4. **checkout:** a detached worktree at the tip (`HOLLOW_NIGHTLY_WORKTREE`,
   reused: `checkout --force`, `clean -ffdx` keeping `node_modules`), then the
   stamp `0.6.<first-parent count>` (`scripts/version.mjs`; a shallow clone
   is refused). A stamp not above the live manifest's version fails the
   `manifest` check: the channel never moves back.
5. **install:** `npm ci`. **gate:** a gate record for the tip's source
   fingerprint (`node scripts/source-provenance.mjs --print`) in
   `<git common dir>/hollow-gate-records/` or `HOLLOW_NIGHTLY_GATE_RECORDS`,
   `gate: "passed"` and not dirty; else `npm run gate` there.
6. **build:** `scripts/release/build-app.mjs --ref <tip> --no-verify` (plus
   `--dmg` with `HOLLOW_NIGHTLY_DMG=1`). **verify:** `--verify-only
   --no-selftest` (codesign, team, `spctl` and `stapler` when notarized,
   version, provenance). **selftest:** `--selftest-only`.
7. **sign:** `tar --no-mac-metadata -czf Hollow.app.tar.gz` (no AppleDouble
   `._` files, which would break the unpacked app's signature), then
   `tauri signer sign`, which writes the `.sig`. The key and password come
   from the environment only; with no password set the argv carries
   `--password ''` (tauri CLI 2.11 has no `--no-password`, and without a
   password argument it would prompt). **archive:** the archive is unpacked
   and that copy, the bytes that ship, must pass `codesign --verify --deep
   --strict`, `spctl --assess --type execute` and `stapler validate`.
8. **notes:** `scripts/release/notes.mjs` from the previous Nightly's commit
   to the tip: PRs (`Merge #N: …` and `Merge pull request #N …`) with their
   build-log "not checked" sentences, direct commits, and the
   `docs/WHATS_NEW.md` entries new since then. `notes.json` holds all of it;
   the manifest's `notes` is the plain-text list cut to 4,000 characters. The
   first Nightly (no previous) lists the newest 20 changes.
9. **manifest:** `channels/nightly.json` in the Worker README's format, URL
   `https://<HOLLOW_UPDATES_HOST>/builds/<v>/Hollow.app.tar.gz`, checked with
   the Worker's `parseManifest` before anything is uploaded. `schema` comes
   from the tip's source (`SCHEMA_SOURCES` in `nightly.mjs`): each index's
   `SCHEMA_VERSION` constant (`searchIndex`, `embedIndex`, `projectIndex`),
   `session_store::SCHEMA_VERSION` for `sessions` and
   `companion_inbox::LEDGER_SCHEMA_VERSION` for `companion` (both 0 while
   their migrations stay additive). U6 compares these with the installed
   build's.
10. **upload:** the live manifest is read once more: if it points at this
    version, nothing is overwritten; if it is newer, nothing moves back. Then
    `publishOrder()` order (archive, `.sig`, DMG when built, the build's own
    copy of the manifest `builds/<v>/manifest.json` (never served; promotion
    to Main copies it, U8), `notes.json`, manifest last), one at a time,
    stopping at the first error.
11. Record the publish in `<state>/state.json`, then **prune**: keep the newest
    `HOLLOW_NIGHTLY_KEEP` versions, the live Nightly and the next lower
    version (its previous; Nightly versions only rise), and from
    `<state>/main-history.json` (a JSON array in publish order, oldest
    first, of the entries promotion appends, U8, or plain version strings)
    the newest
    `HOLLOW_NIGHTLY_KEEP_MAIN`, the live Main and the entry before it (the
    previous Main, not the next lower version). Nothing is deleted when a
    channel manifest can't be read, or when `channels/main.json` exists and
    the history is missing, unreadable or doesn't hold it (missing is not
    "no Mains"). A prune error is logged; the build stays published.

**Time limits:** each step (and `git fetch`) runs in its own process group,
so on timeout the whole group, grandchildren such as cargo, vitest or
notarytool included, gets SIGTERM, then SIGKILL after a grace; the step fails
as "timed out" only once no process of the group is left. Limits, in
`STEP_TIMEOUT_MINUTES` (git 10, install 30, gate 120, build 150, verify 15,
self-test 10, tar 15, sign 5, archive 15 minutes. Local git calls are not in
a group (no children). R2 requests abort after 30
minutes (an upload) or 2 (anything else). The long steps' output streams
straight into the daemon's log; only short ones whose output is parsed are
captured. Logged git and tool errors have URL credentials, query strings and
GitHub tokens removed.

**Failure:** any check that fails stops the run before the first upload
(an upload error midway can leave build files, never the manifest). The log
says `FAILED at <check> on <commit>`, `state.json` records it so the same tip
isn't retried every 5 minutes, and `<outbox>/failure-<commit>.json` holds a
notification in Hollow's `NotificationInput` shape (`title`, `body`,
`kind: "error"`, `source: "release.nightly"`, `notifyNative`) plus `commit`,
`version`, `check`, `buildId`. Hollow has no drop folder for notifications
from another machine yet, so nothing reads the outbox; bringing it to the
MacBook's notification centre is open (U5 or a pull like `mini-audit-pull`).

**Secrets** come from the environment only and reach only their step: the
updater key only `tauri signer sign`; the App Store Connect key only
`build-app.mjs`; the R2 keys only the uploader; `npm ci`, the gate, git and
tar get none. Nothing is printed or put in argv (a test greps for it).

| Variable | Meaning |
|---|---|
| `HOLLOW_NIGHTLY_REPO` | The builder's clone (default: the checkout `nightly.mjs` runs from). Never a dev checkout. Needs full history. |
| `HOLLOW_NIGHTLY_STATE_DIR` | State, lock, outputs (`out/<v>/`), outbox. Default `~/Library/Application Support/hollow-nightly`. |
| `HOLLOW_NIGHTLY_WORKTREE` | The build worktree, outside the repo. Default `<state>/worktree`. |
| `HOLLOW_NIGHTLY_OUTBOX` | Failure notifications. Default `<state>/outbox`. |
| `HOLLOW_NIGHTLY_INSTALL_COMMAND`, `HOLLOW_NIGHTLY_GATE_COMMAND` | Default `npm ci`, `npm run gate`. |
| `HOLLOW_NIGHTLY_WRAP` | A prefix for the gate and build, e.g. `hollow-slot --here` to hold the mini's Rust lock. |
| `HOLLOW_NIGHTLY_GATE_RECORDS` | More gate record folders, `:`-separated. |
| `HOLLOW_NIGHTLY_DMG` | `1`: build and upload `Hollow.dmg` (not notarized on its own yet: U11). |
| `HOLLOW_NIGHTLY_KEEP`, `HOLLOW_NIGHTLY_KEEP_MAIN` | Retention, default 20 and 10. |
| `HOLLOW_NIGHTLY_LOCK_STALE_MINUTES` | Default 360. |
| `HOLLOW_UPDATES_HOST` | Host in the manifest URL. Default `updates.example.com`. |
| `TAURI_SIGNING_PRIVATE_KEY_PATH` (or `TAURI_SIGNING_PRIVATE_KEY`), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The updater key. Without a password, `--no-password` is passed. |
| `HOLLOW_R2_ACCESS_KEY_ID`, `HOLLOW_R2_SECRET_ACCESS_KEY`, `HOLLOW_R2_ENDPOINT`, `HOLLOW_R2_BUCKET` | R2's S3 API (`https://<account>.r2.cloudflarestorage.com`, bucket default `hollow-updates`). |
| `HOLLOW_SIGNING_IDENTITY`, `HOLLOW_TEAM_ID`, `APPLE_API_*`, `CARGO_TARGET_DIR` | Passed to `build-app.mjs` ([Build & ship](reference/build-and-ship.md)). |

**The LaunchDaemon file:** `scripts/release/com.hollow.app-nightly.plist`
runs `nightly.mjs` as `runner` every 300 s (`RunAtLoad` false), logging to
`~runner/Library/Logs/hollow-nightly.log`, after sourcing
`/Users/runner/.config/hollow-nightly/env` (0600, the variables above, one
`NAME='value'` per line: `sh` sources it, so every value must be shell-quoted,
single quotes for anything holding spaces, `$` or quotes). It
is written only; nothing loads it. Before loading, replace `__NODE__` and
`__REPO__` in a copy. To load it (Alex's word, design section 13 default 2):

```sh
sudo install -m 644 -o root -g wheel com.hollow.app-nightly.plist /Library/LaunchDaemons/ && sudo launchctl bootstrap system /Library/LaunchDaemons/com.hollow.app-nightly.plist
```

Stop it with `sudo launchctl bootout system/com.hollow.app-nightly`; one
run by hand: `sudo launchctl kickstart system/com.hollow.app-nightly`.

**Runbook:**

- After a `config` refusal (or any failure you fixed), the same tip is not
  retried on its own: run once by hand as `runner` with
  `node <repo>/scripts/release/nightly.mjs --retry` (the env file sourced
  first), or wait for the next merge.
- **Rewritten `main`** (a force push that lowers the first-parent count): the
  builder refuses to move the channel back (`manifest` check). After
  confirming the rewrite was meant, delete `channels/nightly.json` from the
  bucket by hand (the Worker then serves no Nightly), and the next run
  publishes the new tip. Builds under `builds/` stay until prune.
The daemon runs `nightly.mjs` from the clone's own checkout at `__REPO__`,
which the builder never updates (it fetches only `origin/main` into the
tracking ref and builds in its worktree). So a change to the builder itself
reaches the mini only when that checkout is fast-forwarded: U11 sets up that
step (`git -C __REPO__ merge --ff-only origin/main` by hand or before each
run). Still open for U0/U11: that step, and unlocking the release keychain
for `codesign` per run.

## The updater client (U5)

`src-tauri/src/update_client/` (design sections 2, 7, 8 and 9). PR 1 is the
backend and the typed contract; the top bar's Update button, its popover and
Settings, Updates are PR 2, on these commands only.

**What it does.** About 45 seconds after launch, then every 30 minutes (and on
Check now, a connect or a channel switch), it asks
`<host>/channels/<channel>.json` with `Authorization: Bearer <token>`. An
offered build downloads in the background through `tauri-plugin-updater`
(`~2.12`, the last line on Tauri 2.11; Rust only, no npm package), whose
`download` checks the minisign signature against the public key. The plugin's
`install()` is never called. The archive is unpacked member by member into
`<app data>/updates/staging/<version>/Hollow.app`, then checked with the
installer's own functions (`update_install::verify`: no symlink leaving the
bundle, not inside the installed app, `codesign --verify --deep --strict`, the
Developer ID team, bundle id `com.hollow.app`, version equal to the
manifest's). Only then is the phase `ready`, so the Update button appears only
for a verified download. A `ready.json` beside the bundle lets the next check
(or the next launch) reuse it without downloading again; other versions'
folders are removed.

**Inert until set up.** With no public key (`tauri.conf.json`
`plugins.updater.pubkey`, empty until U11 makes the key pair), no usable host,
or in a review copy, the phase is `notSetUp` with the reason ("Updates are not
set up on this build."); with no token it says how to connect. Nothing panics
and nothing is requested. The empty `plugins.updater` entry must stay: the
plugin refuses to start without one. `bundle.createUpdaterArtifacts` stays
off; the release builder signs the archive itself (U2).

| Setting | Where | Default |
| --- | --- | --- |
| host | `HOLLOW_UPDATES_URL` (a plain origin; tests use a local server), **read only in tests and debug builds** | `https://updates.example.com` |
| signing team | `HOLLOW_UPDATE_TEAM` (`-` for ad hoc), **tests and debug builds only** | `TEAM000000` |
| public key | `tauri.conf.json` `plugins.updater.pubkey` | empty (inert) |

A release build always uses the default host, requires `https`, and refuses
an address with a user name or password (the host, a manifest's download URL,
a redirect).

**Phases** (`UpdatePhase`, tagged `phase`): `notSetUp {reason}`, `idle`,
`checking`, `upToDate {skipped}`, `available {release}`, `downloading
{release, percent}`, `ready {release}`, `installing {release}`, `goingBack
{version}` (U6), `failed {reason}`. A ready download keeps its phase (and button) while the next check
runs; offline, it stays ready with a `note`. `UpdateState` adds
`runningVersion` (`build-provenance.json`), `channel`, `configured`,
`connected`, `waiting`, `goBackWaiting` (U6), `dismissed`, `skippedVersion`, `downgradeAllowed`,
`lastCheckedAt` and `note`. It never carries the token.

**Offer rule** (`state::decide`): a higher version is offered; equal or lower
is not; the `skippedVersion` (set by Go back, U6) never is until a newer one is
published; a lower one only while `allowDowngradeOnce` is set. A channel switch
sets it (and says so in `note`) with `allowDowngradeSinceMs`; it clears when
that install starts (`--allow-downgrade` is passed only then), when the new
channel offers nothing lower or something higher, and 24 hours after the
switch (a switch time in the future counts as expired). An older build whose
permission expired after it was downloaded is not installed: Restart to
update checks the channel again instead. Versions that don't compare are never offered. "Not now"
(`update_dismiss`) hides that version's button until a newer one is ready.

**The manifest** is checked again by the client before any download: its
`channel` must be the one asked, and the download URL must be on the update
host, at exactly `/builds/<version>/Hollow.app.tar.gz` (the token goes with
the download, so it never leaves the host). A 401, 403 and 404 are told apart
(the plugin hides status codes, so after "no release" it asks once more for
the status alone): "did not accept this token (it may have been revoked)"
(U9's words), "can't use the Main channel",
nothing published.

**Downloads** stop past 400 MB (announced or received) and after 25 minutes
in all (a stall between bytes after 60 s), so a trickle can't keep the client
busy; redirects are followed only on the update host (at most 5). Staging
folders are 0700.

**Unpack refuses** absolute paths, `..`, members outside `Hollow.app`, symlinks
that are absolute or resolve outside it, writes through a symlink, hard links,
devices and FIFOs, and archives over 2 GiB or 200,000 members; set-id bits are
dropped. The cap counts each member's PAX size, not the header field.

**Epochs.** A channel switch, a connect and a disconnect bump an epoch
(under the client's lock, so a check's compare-and-apply, prune included, can
never interleave with it): a check that started before drops its answer, a
download stops at its next chunk, and one more check runs. A check clears the
downgrade permission only if no newer switch set it since. A channel switch
and a disconnect are refused while an install is starting. While an install is starting, nothing prunes or discards staged files,
whatever a running check finds. A withdrawn build (the channel now offers
nothing) ends "Update when chats finish". A refused token (401 or 403)
doubles the wait between checks, up to 4 hours; any other answer resets it
to 30 minutes. Check now, a connect and a channel switch wake the schedule
thread, which checks at once and restarts its wait. A later successful check or `ready` clears `note`.

**Commands** (`src/api.ts`, types in `src/generated/ipc.ts`; none is an agent
action: reads are plumbing, the rest the person's clicks, design section 7):

| Command | Answers |
| --- | --- |
| `update_get_state` | `UpdateState` |
| `update_get_settings` | `UpdateSettings` |
| `update_set_channel {channel}` | `UpdateState`, then a check |
| `update_set_overnight {enabled}` | `UpdateSettings`; takes effect at once (U7, Overnight below) |
| `update_check_now` | `UpdateState`; the check runs on its own thread |
| `update_connect {token}` | `{connected}`; then a check (the person typed it: it replaces a stored token) |
| `update_confirm_connect` | `UpdateState`: stores the link's waiting token (`pendingConnect`), then a check |
| `update_discard_connect` | `UpdateState`: forgets the link's waiting token |
| `update_disconnect` | `{connected: false}` |
| `update_dismiss {version}` | `UpdateState` |
| `update_install_now {confirmedKeys?}` | `UpdateInstallOutcome`: `started {version}`, `workRunning {work}` or `workChanged {work}` |
| `update_install_when_idle` | `UpdateState` (`waiting: true`) |
| `update_cancel_wait` | `UpdateState` |
| `update_last_result` | `UpdateLastResult` or null |
| `update_ack_last_result` | nothing |
| `update_go_back_plan` | `UpdateGoBackPlan` (U6, below) |
| `update_go_back {restoreData, confirmedKeys?}` | `UpdateGoBackOutcome`: `started {version}`, `workRunning {work}`, `workChanged {work}` or `refused {reason}` |
| `update_go_back_when_idle {restoreData}` | `UpdateState` (`goBackWaiting: true`); `update_cancel_wait` cancels it |
| `update_unskip` | `UpdateState`, then a check |
| `update_offer_local_build {id}` | `UpdateState`: the build workspace's finished build as the Local build (U10) |

**Events:** `update-state` (an `UpdateState`) on every change, and
`update-last-result` (an `UpdateLastResult`) once at launch when a notice
waits (the window should also ask `update_last_result` when it loads).

**Restart to update** (`update_install_now`): only from `ready`. First the
app verifies the staged bundle again (the helper runs the staged bundle's own
code and can't vouch for itself); then the work check and the claim. Without
`confirmedKeys` nothing may run (`lifecycle::running_work_now` empty and
`self_install::install_blocked` clear); with them, the restart check
(`lifecycle::check_restart_work`) must pass, as `setup_restart` does. Running
work is an answer (`workRunning`, `workChanged`), not an error, so the window
can offer the wait. Then it writes `last-update.json`, starts `<staged>/Contents/MacOS/hollow install-update
--staged … --expected-version … --expected-team … --app <running bundle>
--journal-dir … --snapshot-source <Sessions> (or --no-snapshot) --install-id …
--relaunch [--allow-downgrade]` detached (`update_install::detach`, log
`install-update.log` in the journal folder) and watches it for 500 ms: an
installer that exits at once leaves Hollow running and `ready` with the
reason in `note`. It quits 1.2 s later, after one last look at running work
and at the installer (one that already exited keeps Hollow open):
if something started meanwhile, it stops the installer (which has moved
nothing while Hollow runs), removes `last-update.json` and goes back to
`ready` with a note; from "Update when chats finish" it waits again. A dev run
that is not inside a `.app` refuses (the build stays `ready`).

**Update when chats finish** (`update_install_when_idle`): looks every 5 s,
with **no expiry**, until nothing runs (the same checks), then installs as
above; Cancel (`update_cancel_wait`) stops it. It lives in memory only: if
Hollow quits, the wait is gone. Local builds use the same wait (U10).

**After the restart:** `update_last_result` reads `last-update.json` (never
in the app that wrote it): `updated {version, notes, channel, at, local}` when this is
the new version; `wentBack {attempted, restored, reason, at, manual,
dataRestored}` when the installer's `last-install-failure.json` names this
install (`manual: false`) or after the person's Go back (`manual: true`,
`attempted` the version gone back from); `notWentBack {target, reason, at}`
when a Go back didn't happen; `notInstalled` otherwise. `update_ack_last_result` marks it shown.

**The token** is 43 base64url characters, kept in the Keychain through
`secret_store` (key `updates:token`), so it is in no settings file or export.
`update_connect` takes it once and answers only `connected`; it is never
logged, put in an error, a state or an event (a test greps the module's log
lines; `Debug` on the check's inputs redacts it). **The Keychain is read off
the main thread:** `Client::new` reads nothing; the schedule thread's first
step reads the token (`secret_store` caches its one item per process, but its
first read in a new build can prompt). **Connect link:**
`hollow://updates/connect?token=<token>` (the install page's Connect Hollow
button) is routed in `lib.rs` before the Linear link handler: only `token` is
read; any other parameter, a fragment, a port or credentials refuse it with a
fixed sentence, as a note, without showing a hidden window. The same link
again within 3 s is ignored (its SHA-256 is compared, not kept). With no
token stored, it stores this one and checks; with the same token, nothing
changes; **with a different token stored, the new one waits in memory**
(`pendingConnect: true`, never in IPC, events or files) for the person's
`update_confirm_connect` or `update_discard_connect`. A link that arrives
before the client exists is held in memory for its first tick. Then Settings,
Updates opens. Nothing records the URL.

**Settings** (`update-settings.json` in app data, every field defaulted):
`channel` (`main`), `installOvernight` (false), `skippedVersion`,
`dismissedVersion`, `lastCheckedAt`, `allowDowngradeOnce`,
`allowDowngradeSinceMs`, `dismissedLocalBuild` (U10), and from U9 `personName`,
`allowedChannels` and `publisher` (see People). A file that doesn't parse is moved aside to
`update-settings.json.bad` before the defaults are used (it may hold a skipped
version). Not agent-visible
settings keys yet (`settings_keys.rs`): the channel and the overnight switch
stay Settings clicks.

**Server text is plain text.** A manifest's `notes`, and the reasons in
`failed`, `note` and `UpdateLastResult` (some quote the server or the
installer), must be rendered as plain text, never as HTML or Markdown links.

**Dependencies:** `tauri-plugin-updater` with default features off and
`rustls-tls` only (no `zip`, which is Windows-only, and no `system-proxy`), and
the plugin's own `reqwest` 0.13 named as `reqwest_updater` for its redirect
policy type. Its TLS stack brings `rustls-platform-verifier`, which pulls
`webpki-root-certs` (Mozilla's roots as data, licensed CDLA-Permissive-2.0).
Without `system-proxy`, update traffic ignores the macOS system proxy
settings and goes direct. Intended: the update host is public HTTPS with its
own bearer token, no Mac here needs a proxy to reach it, and a proxy in the
path would see the token's requests. If a teammate's network requires a
proxy, turn the feature back on.

**Tests:** `cargo test --lib update_client`: the offer rule, the phase machine,
the manifest (fixture `src-tauri/tests/fixtures/updates/manifest-nightly.json`),
unpack safety, the client on fakes (inert, connect, download and verify,
refusals, reuse, offline, channel switch and downgrade, install refusal and
arguments, the no-expiry wait on a fake clock, the notice, the token never in
events or files), and the real plugin against a local `TcpListener` with an
archive signed in the test by a throwaway minisign key (Ed25519 through
`ring`, minisign's legacy `Ed` format).

## Publish to Main (U8)

Design sections 9 and 10. Main only moves when Alex publishes: a kept
Nightly is promoted **without a rebuild** (the same signed bytes), and
Withdraw points Main back at the Main before it.

**On the builder: `scripts/release/promote.mjs`.** Runs where the R2 write
credential is (the mini, as `runner`, with the builder's env file):

| Command | What it does |
| --- | --- |
| `status [--eligible v,v] [--pick v]` | the live Nightly and Main, the kept builds that have a record (`builds/<v>/manifest.json`), the Main history, the proposed build (the pick, else the newest `--eligible` version above Main) with what it brings since the live Main (`notes.mjs` from Main's commit, in the builder's clone), how far Nightly is ahead (`changesAhead`, `daysAhead`, `nudge`) and whether Withdraw can go back |
| `list-nightlies` | the kept builds with a record: version, commit, `pub_date` |
| `promote <v> [--expect-commit <sha>] [--expect-main <v\|none>] [--force-older]` | reads `builds/<v>/manifest.json`, checks every `manifestPrerequisites` file is in the bucket and the stored `.sig` equals the record's signature, then writes `channels/main.json` (the record with `channel: "main"` and notes since the previous Main) as its **only and last** write, then appends `{version, commit, promotedAt, from, action: "publish", by}` to `<state>/main-history.json` |
| `withdraw [--expect-version <v>]` | replays the history (a publish pushes, a withdraw pops), checks its top is what `channels/main.json` serves, verifies the Main before it the same way, writes `channels/main.json` for it, appends an `action: "withdraw"` entry |

Rules (decided 2026-11-15): promote refuses the version Main already serves,
without `--force-older` one lower than the live Main (Withdraw is the way
back; Hollow never passes it, and its card refuses an older build), and with
`--expect-main` a Main that moved since the caller looked. Withdraw never
needs `--force-older` (going back is its job) but refuses with no earlier
Main (an empty Main would offer teammates nothing) and with
`--expect-version` when Main moved. **Crash repair:** `channels/main.json`
is written before the history, so a run stopped between them leaves Main
ahead of the history. The next promote or withdraw (`reconcile`) adds the
missing entry first (a live Main one below the history's top is a withdraw;
any other live Main must be a kept build's record with the same commit, a
publish), and promote of the version Main already serves then adds only
that entry (`repaired: true`). A live Main with an **empty history** (seeded
by hand, or its build record pruned) is recorded as the base of the stack
(`seeded: true`); Withdraw to it still needs its record. Any other
`main.json` no kept record accounts for is refused (`history`: fix by hand). Promotions and withdraws take
`<state>/promote.lock` (not the Nightly lock, so a 25-minute build doesn't
block a publish). Output: progress on stderr, the result as one
stdout line `HOLLOW_PROMOTE_RESULT <json>` (`ok`, or `code` and `error`);
exit 0 done, 1 refused, 2 usage or configuration. **Builds published before
the record existed can't be promoted** (none had been when U8 landed).

**Prune and promotion:** the Nightly builder prunes under the same
`promote.lock` (skipped, with a log line, while a publish or withdraw holds
it), so neither moves Main between prune's reads and its deletes. Prune's
Main versions are the replayed stack (`replayMainStack` in `nightly.mjs`), so
the previous Main is the one Withdraw would go back to, never a withdrawn
build.

**From Hollow: `update_client/publish.rs`.** One ssh call to this Mac's
runner (`remote_session::configured_runner`, with remote sessions' ssh
options) runs `remote_command`: source `~/.config/hollow-nightly/env`, fetch
`origin/main` into the builder's clone (`$HOLLOW_NIGHTLY_REPO`, default
`~/hollow-nightly/repo`), `git archive` its `scripts/release` (and the files
they import) into a temporary folder, and run `promote.mjs` from there. So
the script is always main's, whatever the clone's checkout is, and nothing
secret crosses the connection. The fetch is stopped after 60 s, and a failed
fetch ends the call with code `fetch` (nothing runs from a stale checkout).
The ssh call itself goes through `remote_status::run_bounded` and is killed
after 120 s (the message says a publish may still have finished there). Its
stderr is redacted (`claude_runner::redact`) before it reaches a reason. A
result counts as done only with exit 0, `ok`, a release version, a time and
an action; anything else is `refused {code: "failed"}`. On the work tailnet
the runner must be `runner-host-lan` (`~/.config/hollow/slot-remote`).

- **Who publishes:** `UpdateSettings.publisher` ("This Mac publishes to
  Main", off by default; People, U9, reads the same field). Off: no command
  calls the runner and Publish and Withdraw are refused.
- **The proposed build:** the newest Nightly that ran on this Mac at least an
  hour in one stretch without a go back. `<app data>/updates/run-history.json`
  (`{runs: [{version, firstSeenMs, lastSeenMs}]}`, last 60 stretches) is
  written at launch and every 30 minutes; a silence over 65 minutes (about
  two ticks: quit or asleep) starts a new stretch. A go back is the `skippedVersion`
  (U6) or any stretch followed later by a lower version.
- **The nudge:** on a publisher Mac, at most every 6 hours, the tick asks
  `status`; when Nightly is 3 days (the oldest build Main lacks) or 20
  changes (PRs and direct commits since Main's commit) ahead, one Hollow
  notification per Main version it compares against (a new Nightly every
  night would otherwise nudge daily) and never two within 24 hours: "Nightly
  is N changes ahead of Main: review and publish" (action Review,
  `settings:updates`). Memory: `<app data>/updates/publish-nudge.json`
  (`nudgedFor`, `nudgedMain`, `lastNudgeMs`, `lastCheckMs`). A runner that doesn't answer is
  asked again at the next tick.

| Command | Answers |
| --- | --- |
| `update_publish_status {pick?}` | `UpdatePublishStatus` (read; agent action `build.publish_status`) |
| `update_publish_main {version, commit, expectedMain}` | `commit` (required) and `expectedMain` (null: no Main yet) are what the card showed; `UpdatePublishOutcome`: `published {version, from, at, historyWarning}` or `refused {code, reason}`. The Settings button is the person's click; agents reach it only through `build.publish_main` (always asks) |
| `update_withdraw_main {expected?}` | `UpdatePublishOutcome`: `withdrawn {version, from, at, historyWarning}` or `refused` (user-only) |
| `update_set_publisher {enabled}` | `UpdateSettings` (user-only) |

**The agent action `build.publish_main`** (External, `ALWAYS_ASK`: a card in
every mode, Bypass included; automations and Main check-ins never run it).
An agent proposes a version; the card names it, its commit, the live Main
and the PR titles since Main, and binds `{version, commit, main}`; a build
older than the live Main gets no card. Only the person's Allow publishes,
with `--expect-commit` and `--expect-main` from the card. Withdraw has no
action.

**The card** (`src/settings/UpdatesPublish.tsx`, lazy, shown only while
`publisher` is on): asks `update_publish_status` when it opens, on
**Refresh** and when the **Build** picker changes (the pick), never on a
timer, with a spinner meanwhile. It shows Main, Nightly, how far ahead, the
builds (ran here, not newer than Main) and what's new since Main. **Publish
0.6.n to Main** (off for a build without a commit or not newer than Main)
and **Withdraw 0.6.n** each ask first and send the commit and Main the card
showed; the outcome line words `published`, `withdrawn` or `refused` and any
`historyWarning`, then the status is read again.

**Tests:** `node --test scripts/release-promote.test.mjs` (fake bucket: files
and signature verified first, `channels/main.json` the only and last write,
the history appended, refusals write nothing, withdraw twice then refused,
crash repair, `--expect-main`, busy lock, status's proposal and nudge, each
release script run through a symlinked path), `scripts/release-nightly.test.mjs`
(prune by the replayed stack and under the promote lock) and `cargo test
--lib update_client::publish_tests` (run history and its gap, the proposal
rule, the runner contract on a fake runner, malformed or failing results,
redaction, a failed fetch under `/bin/sh`, quoting, the nudge's dedupe and
24-hour gap, the card, its binding and the older-build refusal, asking in
every mode).

## People (U9)

Design section 11: Alex adds teammates, each gets a token and an install
link, and each teammate's channel picker offers only what he allows.

**The publisher's Mac.** `UpdateSettings.publisher` (off by default, set by
`update_set_publisher`, U8's command in `publish.rs`) shows People in
Settings, Updates. Every People command refuses on any other Mac.
`people.json` is in the R2 bucket and the only R2 write credential is the
release builder's, so each change runs `scripts/release/people.mjs` on this
Mac's remote runner (`~/.config/hollow/slot-remote`, the Mac mini): one `ssh`
with the remote sessions' options (no prompts, no agent forwarding) running
`/bin/sh -c '<script>' people <words>`. The script sources
`~/.config/hollow-nightly/env` (the builder's env file: `HOLLOW_R2_*` and
`HOLLOW_NIGHTLY_REPO`, default `~/hollow-nightly/repo`) and runs
`people.mjs` from that clone, so a change to the script reaches the mini when
that clone is fast-forwarded, as for `nightly.mjs`. Not through `hollow-slot`:
it gives the command no stdin and sends a checkout. One People call at a time
from this Mac (90 s limit each); only the script's own `people: ...` error
line is ever repeated. The ssh adds `StrictHostKeyChecking=yes`: the runner
must already be a known host. Each answer must match the change asked for
(`revoke`: the same id, `revoked: true`; `set-channels`: the same id and
exactly the channels sent; `add`: an id `p-<12 hex>`, the same name and
channels, not revoked), or the call fails with "doesn't match the change asked
for" and `add` returns no link.

| Command | Answers |
| --- | --- |
| `update_people_list` | `UpdatePeopleList {people: UpdatePerson[]}` |
| `update_people_add {name, channels}` | `UpdatePersonAdded {person, installLink}` |
| `update_people_revoke {id}` | `UpdatePerson` (`revoked: true`) |
| `update_people_set_channels {id, channels}` | `UpdatePerson` |

`UpdatePerson` is `{id, name, channels, revoked, createdAt}`: never a token or
a hash. All four are user-only (`COO_EXCEPTIONS` give why), as is the switch.

**Adding a person.** The app makes 32 bytes with the OS random source
(`OsRng`), encodes them as 43 base64url characters, and sends only the
lowercase hex SHA-256 to the runner, **on ssh's stdin** (`people.mjs` refuses
a hash-shaped argument). The token comes back to the window once, inside
`installLink` (`https://updates.example.com/install/<token>`), and is kept
nowhere: not in settings, state, an event, History or a log, and the answer's
`Debug` redacts it. `UpdatePersonAdded` isn't `Clone`, and its link and the
token are overwritten when dropped (best effort). The window shows it once
with Copy; a lost link means revoke and add again. A name is 1 to 80
characters with no control, format (`\p{Cf}`: the BOM, zero widths, bidi
overrides) or separator characters, checked before trimming on both sides
(JS's `trim` strips U+FEFF, Rust's doesn't); a new person needs at least one
channel (Main, Nightly or both).

**`people.mjs`** (`add <name> <channel...>` with the hash on stdin, `revoke
<id>`, `set-channels <id> [channel...]`, `list`, `init`) reads `people.json`
and its ETag, checks it with the Worker's own `parsePeople` (an invalid file
is never written over), changes the raw entry, checks the whole result
again, then writes it with one conditional PUT (`r2-uploader.mjs`):
`If-Match` on the ETag read, so a change made in between by another writer is
a 412 and "people.json changed while this ran, so nothing was written"
instead of a lost change. A missing `people.json` is created only by `init`
or `add --init` (`If-None-Match: *`); otherwise every command refuses with
"not in the bucket; check HOLLOW_R2_BUCKET", so a wrong bucket setting can't
quietly start a new list. The app never passes `--init`: run `people.mjs
init` once on the builder. New ids are `p-<12 random hex>`. **Revoke only
sets `revoked: true`**: entries are never deleted, so the file is the audit
trail (and a revoked person can't be given channels again; add them anew).
A revoke (or a channel change) reaches the Worker within its 60 s
`people.json` cache. It prints one JSON line and never a hash.

**A teammate's Mac.** After a connect and on every check, before the
channel, the client asks `GET /me` and keeps the answer in
`update-settings.json` (`personName`, `allowedChannels`); `UpdateState` adds
`allowedChannels` (both until `/me` answers, `[]` for none) and `personName`.

- `update_set_channel` to a channel not in `allowedChannels` is refused with
  `CHANNEL_NOT_ALLOWED: Your updates access doesn't include the <Channel>
  channel.` and changes nothing.
- A channel that stops being allowed moves to the first allowed one (Main
  before Nightly) in the same check, with a `note` that stays until the
  person picks a channel, disconnects, or the channel is allowed again. The
  move sets the channel and the note only: unlike the person's own switch it
  never arms the one-time downgrade (and disarms one left armed), so a `/me`
  answer can't get an older build offered.
- No channel allowed: `failed` with "Your updates access doesn't include any
  channel right now..." and no channel request.
- **A revoked token fails at its next check:** `/me` (or the channel) answers
  401 and the phase is `failed` with "The update server did not accept this
  token (it may have been revoked)..."; the schedule's refused-token backoff
  doubles the wait up to 4 hours. The Worker answers one identical 401, so
  the app can't tell revoked from unknown and says so.
- `/me` 404 (a Worker from before U9) or offline: what was known stays. Only
  `name` and `channels` are read; other fields (a newer Worker's) are ignored
  and never kept, as unknown channel names are.
- A new token (`update_connect`, a confirmed link) or a disconnect forgets
  `personName` and `allowedChannels`.

**The card** (`src/settings/UpdatesPeople.tsx`, lazy): "Connected as
{personName}", the **This Mac publishes updates** switch
(`update_set_publisher`), and on the publisher's Mac the People list
(Main and Nightly ticks per person, **Revoke**, asked first), **Add person**
(name, channels, asked first) and **Refresh**. One call at a time with a
spinner (up to 90 s each). The install link is shown once with **Copy link**
and **Done**, held only in the card's memory and cleared when it closes.
The page's **Channel** picker offers only `allowedChannels` and is off when
there are none.

**Tests:** `node --test scripts/updates-worker.test.mjs` (`/me`) and
`scripts/release-people.test.mjs` (a fake bucket), and `cargo test --lib
update_client::people_tests` (`/me` parsing and a one-shot local server, the
fallback, the refusal, revoked words, token shape and a bit-count sanity
check, the hash only on stdin, the link in no file, shell quoting through a
real `/bin/sh`).

## Overnight (U7)

`update_client/overnight.rs`, design section 8. **Install updates overnight**
(`installOvernight`, off by default) starts nothing new: a thread
(`hollow-updates-overnight`, not in a review copy) looks every 10 minutes and
reads the setting at each look, so turning it on takes effect at once
(`update_set_overnight` wakes the thread) and turning it off means the next
look installs nothing. It installs the build in the Update button when **all**
of these hold (`overnight::decide`, a pure table):

- the phase is `ready` with a **channel** update: never a Local build (a build
  someone staged by hand), nor one from another channel or a channel the
  token may no longer use;
- not the `skippedVersion`, not the version hidden with **Not now**, no link
  token waiting for the person (`pendingConnect`) and no go back waiting;
- local time 2:00 up to 5:00 AM (`chrono::Local`);
- nothing runs or waits: the same `Work` checks as Restart to update
  (`lifecycle::running_work_now`, live and queued rows included, and
  `self_install::install_blocked`), a held `hollow-slot` build slot or cargo
  lock (the dev-server lock doesn't count: a leftover one would hold every
  night), or a self-hosted job draining; a list that can't be read counts as
  work;
- no run started in the last 10 minutes (`run_guard::begin_tracking` stamps
  every admitted chat turn, automation and build workspace turn);
- on power: `pmset -g ps` says `'AC Power'` (a Mac with no battery does);
  battery or an unreadable answer waits. Read last, so nothing runs `pmset`
  by day.

Otherwise it looks again in 10 minutes; past 5:00 it waits for the next
night. The install is `Client::install`, the manual path armed for a channel
update only, so the staged bundle is verified again before anything spawns,
the restart checks and the last look before the quit apply, and an older
build whose one-time downgrade permission expired is not installed. It is
handed the version the look decided on, and under its lock, both before
verifying and at the claim, reads again that the button still holds that
version as a channel build, the setting is on, it isn't skipped or hidden with
Not now, and the channel is unchanged and allowed (`overnight::recheck`);
otherwise the look holds and nothing spawns. It marks `last-update.json`
`overnight: true`. A refusal (a failed check, a dev run) is logged and the
phase shows it, as for a click. The thread keeps looking after an install
starts (the quit ends it): when the last look before the quit calls the
install off because work started, the build is ready again and a later look
in the window installs it; no "when chats finish" wait is armed, since that
would install at any hour. No agent action reaches any of this.

**Morning notification:** at the next launch, `last::overnight_notice` posts
one Hollow notification (source `updates.overnight`, opens Settings, Updates):
"Hollow updated overnight to <version>" with the notes as plain text (cut at
1500 characters), or that the update to <version> was rolled back or didn't
install, with the reason (the home folder written `~`, cut at 300
characters). The record's `overnightNotified` is set before the
post, so it never repeats; the window's own notice is unchanged.

**Tests:** `cargo test --lib update_client::overnight_tests`: the rule's table
(window edges, battery, unreadable power, work, a start 5 minutes ago,
skipped, Not now, Local build, other channel, pending token, setting off),
looks on the client fakes (an install with the same arguments and record, each
condition holding with no spawn, a failed codesign refused before any spawn),
the thread (looks every 10 minutes, installs once, wakes at the switch, a
called-off last look installed by the next look with no wait armed), the
recheck (a build replaced during the look, and a skip, Not now, channel switch,
channel no longer allowed or the setting off between verify and the claim,
each refused with no spawn) and the notification (once, only for an overnight
install, failed and rolled back wording, the reason cut and without the home
folder).

## Go back (U6)

`update_client/goback.rs` (the plan), `goback_run.rs` (the flow and
commands) and `update_install/restore.rs` (the data restore), design section
9. The Settings, Updates card is `src/settings/UpdatesGoBack.tsx` (lazy):
it reads the plan on open and after each action, shows `disabledReason`, the
`needsRestore` summary and what is lost, and offers **Go back to 0.6.m** or,
for a newer store, **Go back and restore data from before the update**
(off without an intact snapshot). Each asks first. `workRunning` lists the
chats with **Go back when chats finish** and **Stop these and go back** (the
keys as `confirmedKeys`, asked again); `goBackWaiting` shows **Cancel going
back** (`update_cancel_wait`); `skippedVersion` shows "Skipping 0.6.n" with
**Undo**. The page's status line names phase `goingBack`, and the last
result line words `wentBack`/`notWentBack` (`updatesText.ts`).

### What an older binary does with a newer database

Row U6's first test (`update_client/goback_store_tests.rs`) opens a fixture
stamped with a `PRAGMA user_version` above what the code knows, through each
store's real open path:

| Store | What this build does with a newer one |
| --- | --- |
| Sessions (`Sessions/*.db`) | **Opens it blindly.** `session_store::open` never reads `user_version`; it applies its `CREATE TABLE IF NOT EXISTS` and `ensure_column` and reads and writes as usual. Unknown tables and columns are left alone, and rows it writes get a newer column's default. Nothing refuses or warns. |
| Companion ledger (`Companion/applied.db`) | **Opens it blindly**, the same way. |
| Keyword index (`SearchIndex/*.db`) | Refuses it (`SearchIndex::open` is `None`), leaves rows and stamp: search scans the vault. |
| Meaning index (`EmbedIndex/*.db`) | Refuses it and leaves it, the same. |
| Projects content index | Its reader wants exactly its own version and its writer refuses a newer one: off, left as it is. |

So today an older build reading a newer Sessions database works only because
every Sessions migration so far is additive. Since the U6 review both stores
stamp their own constant on open (`session_store::SCHEMA_VERSION`,
`companion_inbox::LEDGER_SCHEMA_VERSION`, through
`hollow_databases::stamp_schema_version`: raised, never lowered), both 0;
raise one in the same change as a migration an older build would misread
(a dropped or renamed column, a NOT NULL column without a default, a changed
meaning). `goback::known_schema` reports those constants (a test ties them).
A stamp above what the older build knows would mean a change it can't see,
and it would misread silently: **Go back treats a newer Sessions or companion
store as incompatible** and offers only the restore, and only when the
snapshot holds that store: the snapshot copies the Sessions databases only,
so a newer companion ledger refuses Go back with a plain reason. A newer index is never a
blocker: it is rebuildable, and the plan just says the older version leaves
it off until the next update.

### The plan (`update_go_back_plan`)

Reads only. `available` is false, with `disabledReason` in plain words, when:
this is a review copy; the app isn't running from `/Applications`; there is no
`.Hollow.app.previous` ("There's no previous version on this Mac yet. Hollow
keeps one after its first update."); the previous copy's Info.plist can't be
read or isn't `com.hollow.app`; the previous copy isn't older than this one
(after a go back, `.previous` holds the newer version); the verdict needs the
restore and no intact snapshot exists, or a newer store isn't in the snapshot
(the companion ledger); an install or go back is starting here
(`installing`/`goingBack`); or another installer holds the lock.

- `previous {version, installedAt}`: from its Info.plist and the newest
  settled install journal that put that version in place (`promoted` time).
- `runningInstalledAt`: the same for this version.
- `snapshot {at, files, intact, problem}`: the Sessions snapshot of this
  version's install, only when that install came from `previous`; `intact`
  when every copy passes `quick_check` at its recorded version (cached per
  copy while its size, modification time and inode are unchanged, since the
  plan is read on every visit and before each Go back; the helper checks
  every copy in full again). `restoreAvailable` is `intact`.
- `schema`: `compatible`, or `needsRestore {newer, lost, summary}`. Every
  store (`hollow_databases::list` of the support folder) is compared with
  what the previous build knew: `<installer>/known-schemas.json` (each build
  records its own at launch: `sessions`, `companion`, `searchIndex`,
  `embedIndex`, `projectIndex`, the manifest `schema` keys; the newest 30
  versions are kept), else for Sessions the snapshot's recorded
  `user_version`. A blind store above that (or above 0 when nothing is known)
  is in `newer`. `lost {since, chats, messages, summary}` counts, read-only
  from the live Sessions databases, the chats started and messages sent since
  the snapshot (`julianday` compares the stored times).
- `indexesOff`: one sentence per index the previous version would leave off.
- `willSkip` (this version), `skippedVersion`, `waiting` (a Go back wait).

The manifest's `schema` (U2) is not kept by the client; the launch record
holds the same keys and is written by the build itself.

### The flow (`update_go_back`)

Refused (`refused {reason}`, nothing changed) when the plan isn't available,
when `restoreData` is false and the verdict is `needsRestore`, or when
`restoreData` is true and the snapshot isn't intact. Otherwise, as Restart to
update: the previous copy passes `codesign --verify`; the installed app is
cloned to `<installer>/go-back-helper/<id>/Hollow.app` and the clone checked
(signature, bundle id, this version, same executable bytes), since the
helper must not run from a bundle it swaps; then the work check (no
`confirmedKeys`: nothing may run; with them, `lifecycle::check_restart_work`),
answering `workRunning`/`workChanged`; then the claim (phase `goingBack`),
the skip (`skippedVersion` = this version), `last-update.json` with `kind:
goBack` and `restoreData`, and `hollow install-update --go-back --app …
--journal-dir … [--restore-snapshot <snapshot folder>] --install-id …
--relaunch`, detached, from the clone, watched 500 ms. Hollow quits 1.2 s
later after one more look at running work and the helper; either one undoes
the claim, the skip and the record and leaves a `note`.

**Go back when chats finish** (`update_go_back_when_idle`): the same
no-expiry wait as Update when chats finish (`WaitAction::GoBack` on one shared
loop, `wait_loop_for`), shown as `goBackWaiting`. Only one wait runs:
starting one cancels the other kind. A check that finds nothing, a channel
switch and a disconnect end only an update's wait; `update_cancel_wait` ends
either.

### The skip

The version gone back from is `skippedVersion` (U5's offer rule then offers
it only when a newer one is published: skipped 0.6.20 and published 0.6.20 is
not offered, 0.6.21 is). **Undo** (`update_unskip`) clears it and checks, so
0.6.20 is offered again. The overnight installer (U7) reads the same
`update-settings.json` field and never installs the skipped version. At launch
(`settle_go_back_skip`): a Go back that didn't happen clears its skip; the
automatic go back after two failed starts skips the version that failed.
That runs once per `last-update.json` record (its `skipSettled` flag),
whether or not the window already acknowledged the notice. `update_unskip`
is refused while a go back is starting (phase `goingBack`).
Builds before U6 don't know `kind: goBack` and would show "Updated to"
for a go back to them; every build the updater can install has U6.

### Restoring the data

The helper's side is `--restore-snapshot` ([Build & ship](reference/build-and-ship.md),
"In-app installer"): checked copies laid beside the live databases, the live
files renamed to `<file>.before-go-back` and the copies into place, journaled
with the swap so any failure or crash undoes all of it (exit 12 when the
restore can't be done; nothing changed). The chats from after the update stay
in the `.before-go-back` files. A go back that changed nothing (exit 1, 4, 10
or 12 after Hollow quit) opens the installed app again, so the person isn't
left with Hollow closed; it then shows `notWentBack` with the journal's
reason.

