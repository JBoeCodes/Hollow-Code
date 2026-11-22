# Headless Hollow on the Mac mini (`hollow serve` as a LaunchDaemon)

The runbook for running Hollow's core with no window on the Mac mini (M1 PR 4,
2026-11-15). What `hollow serve` is and how it starts:
[Core startup and `hollow serve`](reference/architecture.md#core-startup-and-hollow-serve).
The smoke that proved it on the mini is in the build log (2026-11-15, M1 PR 4).

Nothing here is installed or loaded by an agent. Loading a LaunchDaemon needs an
admin's `sudo`; every `sudo` line below is for the person to run at the mini
(as `builder`, who has sudo) or over `ssh mini`.

## What runs where

- **The daemon:** `/Library/LaunchDaemons/com.hollow.app.serve.plist` (root:wheel
  0644), label `com.hollow.app.serve`, from the template
  [`docs/launchd/com.hollow.app.serve.plist`](launchd/com.hollow.app.serve.plist).
  launchd runs it as `runner` (group `staff`), never as `builder` or root.
- **Why a daemon:** the stock app never starts its core without a window server
  (M0), and a LaunchAgent would wait for a console login. A daemon starts at boot
  as soon as FileVault is unlocked, with nobody logged in.
- **Files**, all in `runner`'s home:

| Path | What |
|---|---|
| `~runner/hollow-serve/bin/hollow` | the serve binary: a copy of `Hollow.app/Contents/MacOS/hollow` from a release build (arm64). Serve needs only this file, not the `.app`. |
| `~runner/hollow-serve/config.json` | `{"vaultPath": "<the mini's own vault clone>"}` (camelCase; the app's config shape). Passed with `--config`; a missing file exits 78. |
| `~runner/hollow-serve/data/` | serve's data folder (`--data-dir`): Sessions DB, `phone.json`, `secrets.json`, `serve.json`, `serve.sock`, locks, caches. Also the daemon's working folder. |
| `/var/log/hollow-serve.log` | stdout and stderr, owned by `runner`, 0600. In a root-owned folder on purpose: launchd may open the log as root before it switches to `runner`, so a path in `runner`'s home would let that user plant a symlink and have root write anywhere. The sudo steps create it; launchd does not create or rotate it. |
| `~runner/hollow-serve/tmp/` | the daemon's `TMPDIR` (0700, made by the script). |

The vault must be the mini's own copy (a clone), never a folder this MacBook's
Hollow also runs: two Hollows on one vault fight over its run queue (serve exits
75 when it finds the queue owned).

The agent CLIs are `runner`'s own: `claude` in `~/.local/bin` and `codex`,
`gh`, `node` in `/opt/homebrew/bin` (the plist's `PATH`; serve also reads the
login shell's PATH at start). Their logins are files (`~/.claude/.credentials.json`,
`~/.codex/auth.json`), so they work with no Keychain. Note they are the Acme
work accounts (`~/CLAUDE.md`): turns there bill Acme's plans.

## One-time setup

1. **Binary (on this MacBook):** after a release build, copy the binary over as
   `runner`:

   ```sh
   ssh runner-host-lan 'mkdir -p ~/hollow-serve/bin'
   scp <build>/Hollow.app/Contents/MacOS/hollow runner-host-lan:hollow-serve/bin/hollow.new
   ssh runner-host-lan 'mv ~/hollow-serve/bin/hollow.new ~/hollow-serve/bin/hollow && ~/hollow-serve/bin/hollow serve --help | head -1'
   ```

   (`ssh runner-host` on the personal tailnet, `runner-host-lan` on the home LAN.)
2. **Config:** write `~/hollow-serve/config.json` with the vault clone's path.
3. **Plist (as `runner`, from a checkout or a copy of `docs/launchd/`):**
   `sh docs/launchd/write-serve-daemon.sh`. It fills in the template for the
   user running it (its home read from the directory, and refused when `$HOME`
   differs), lints it, creates the data folder (0700) and `tmp/` if missing, writes
   `~/hollow-serve/launchd/com.hollow.app.serve.plist`, says what is still
   missing and prints the commands below. It never runs `sudo` or `launchctl`.
4. **Load (the person, with sudo):**

   ```sh
   sudo touch /var/log/hollow-serve.log
   sudo chown runner:staff /var/log/hollow-serve.log
   sudo chmod 600 /var/log/hollow-serve.log
   sudo install -m 0644 -o root -g wheel ~runner/hollow-serve/launchd/com.hollow.app.serve.plist /Library/LaunchDaemons/com.hollow.app.serve.plist
   sudo launchctl bootstrap system /Library/LaunchDaemons/com.hollow.app.serve.plist
   sudo launchctl print system/com.hollow.app.serve | grep -E 'state|pid|last exit'
   sudo tail -f /var/log/hollow-serve.log     # wait for "[serve] ready: ..."
   ```

## Secrets

Serve keeps Hollow's secrets in `data/secrets.json` (0600), not the Keychain
(`runner` has no default keychain). Values go in on stdin, never as an
argument; run these as `runner`, with serve running or not:

```sh
H=~/hollow-serve/bin/hollow; D=--data-dir=$HOME/hollow-serve/data
$H serve secret-set tracker:linear $D        # prompts with no echo at a terminal
$H serve secret-set runner.github.token $D < token.txt   # or any pipe; then delete token.txt
$H serve secret-set claude:<account id> $D   # a token for a Claude account Hollow already lists
$H serve secret-list $D                      # names only
$H serve secret-delete tracker:linear $D
```

`--data-dir` takes `--data-dir <dir>` or `--data-dir=<dir>`. None of these is
needed for a chat: with no Claude account token Hollow runs the `claude` CLI on
its own file login (the smoke ran that way). Adding a new Claude account needs
the app's account list, which serve doesn't have yet. Since A2 (2026-11-15) a paired
MacBook reads serve's `GET /v1/machine/usage`: its Claude Code login and its Codex
sign-in show there as one account each (both the Acme work accounts here),
so the MacBook's spreading steers clear of the one serve just used
(`docs/phone-api/machines.md`).

## Phone API and pairing

Serve answers the phone API when `phone.json` says it is on:

```sh
$H serve enable-phone $D          # in the running serve, or for its next start
$H serve set-default-mode plan --agent claude $D   # a phone's new chat starts in Plan (the default anyway)
$H serve pair $D                  # needs a running serve; prints the code once, good 10 minutes
```

Then `POST /v1/pair` with `{"code", "deviceName"}` from the device
([Pairing](PHONE_CHAT_API.md#pairing)). Another Mac pairs through its own
window instead: `$H serve pair --machine --json $D` (it needs no
`enable-phone`; serve starts listening for it) and `POST /v1/machines/pair`
over an SSH tunnel ([machines.md](phone-api/machines.md)). The server binds `127.0.0.1:47680` only
(no GUI Hollow runs on the mini, so the default port is free; `--phone-port`
moves it).

## The runner COO

Serve runs a COO only when this Mac is the runner (M3b): the `main` workspace's COO
in the mini's own vault and Sessions DB, the same code as the app's. Turn it on as
`runner` (through the running serve, or on disk for its next start):

```sh
$H serve runner-coo on $D        # writes data/runner-coo.json (0600); makes sure workspaces/main exists
$H serve runner-coo status $D    # "Runner COO: on, chat <key>" (no thread yet before its first turn)
$H serve runner-coo off $D
```

On, it is woken when a chat it started ends and at its own `sessions.wake_me`
follow-ups, and it runs its swarms; it never checks in (a check-in asks the person
with cards, which serve refuses, and the person's Mac checks in). Off (the default,
no file), a COO already working in that vault is still woken the same way, but
serve doesn't create its workspace, tells no peer about it and never checks in.
Each role's prompt says how it is run. The app never reads the file: a Mac's COO
is unchanged, runner or not. Serve logs the role at start (`[serve] runner COO: ...`),
with a WARNING when the vault looks synced (a git checkout, iCloud Drive or a cloud
folder): both COOs then write `workspaces/main`. Their wakes never cross: each
Hollow fires only its own (its own Sessions DB and wake file), and a follow-up
carries the Mac it was asked on, so a wake file copied with an app data folder
doesn't fire on another Mac (one an hour past being late is dropped as missed, so a Mac whose hardware id changed gets its conversations' wakes back). `runner-coo status` also shows a record serve could
not read (the role is then off). Messages reach it at its chat key through
the phone API and the peer link (a paired Mac's dispatcher COO routes work there). The COO's status
(`sessions.main_agent_status`) says `role` (`single`, `runner`, `off`) and, on
the runner, `runnerChatKey`; `GET /v1/machine` adds the `coo` capability and that
key (`runner_coo::capabilities`, `peer_coo`), which makes a paired Mac's COO the
dispatcher (M3a, [COO.md](COO.md#the-dispatcher-coo-m3a-2026-11-15)).

## Health check

From this MacBook, through a tunnel to a free local port (this Mac's own Hollow
holds 47680). The `Host` header must name the **server's** port, or it answers
421 `misdirected`:

```sh
ssh -N -L 47681:127.0.0.1:47680 runner-host-lan &
curl -s -H 'Host: 127.0.0.1:47680' http://127.0.0.1:47681/v1/health   # {"ok":true,"app":"hollow","api":1}
kill %1
```

On the mini itself: `curl -s http://127.0.0.1:47680/v1/health`. Process and
memory: `ps -o pid,rss,%cpu,etime -p $(python3 -c 'import json;print(json.load(open("/Users/runner/hollow-serve/data/serve.json"))["pid"])')`
(idle in the smoke: about 85 MB RSS, 0.0% CPU).

## Day to day

- **Update:** copy the new binary to `bin/hollow.new`, `mv` it over `bin/hollow`
  (a rename, so the running one is untouched), then
  `sudo launchctl kickstart -k system/com.hollow.app.serve`. The old process
  gets SIGTERM, settles live runs and exits 0; launchd starts the new one.
- **Stop for a while:** `sudo launchctl kill SIGTERM system/com.hollow.app.serve`.
  A clean stop exits 0 and `KeepAlive` (`SuccessfulExit` false) leaves it
  stopped until `kickstart`. A crash (non-zero) restarts after 60 s
  (`ThrottleInterval`).
- **Exit statuses** (`last exit code` in `launchctl print`): 0 stopped cleanly;
  75 another Hollow owns the data folder or the vault's run queue; 78 no usable
  `config.json` or vault; 1 the runtime could not start; 2 bad arguments. Every
  non-zero exit, 75, 78 and 2 included, is respawned every 60 s under
  `KeepAlive` `SuccessfulExit` false, and each try logs a line or two: a bad
  config left for a day is about 1,500 lines, so fix it or `bootout` (which
  stops the retries), and rotate the log. The daemon starts by itself once the
  cause is fixed.
- **After a mini restart:** FileVault is on and auto-login is off, so someone
  types the FileVault password at the mini (an update from System Settings does
  an authenticated restart instead). Once the disk is unlocked the daemon starts
  at boot, with no login.
- **Log:** `sudo tail -f /var/log/hollow-serve.log` (or as `runner`, who
  owns it). Not rotated: when it grows, `: > /var/log/hollow-serve.log` as
  `runner` (launchd appends, so this is safe while it runs).

## macOS privacy grants after an update

Guidance from Orca's notes on the same trap (OC11), not reproduced on the mini, so check before relying on it. macOS
charges a child process to its app's **original path**, so an update can leave a helper with a **stale denial**: a
permission pane that shows Hollow allowed while the helper is still refused. Accessibility and Screen Recording granted
to the old path need a re-grant after an update. For the daemon, the safe shape is the one above: a **signed copy** of
the binary at a fixed path (`~runner/hollow-serve/bin/hollow`) **started by launchd**, never started by an app
window, so its attribution doesn't follow whichever app version is installed. Serve has no window session, so it can't
answer a privacy prompt itself: a grant (if one is ever needed) is made by the person at the mini's console, and
repeated when the copy at that path is replaced. Update by the rename and `kickstart` in Day to day, then run
`lsof +D <old folder>` on any folder a helper was started from before the update to find what still holds it. The same
note, with the installer's side, is in
[build-and-ship.md](reference/build-and-ship.md#macos-privacy-grants-after-an-update).

## Not provided yet

- Cards. Since M1 PR 5 Hollow app tools run in serve: the COO's reads and the
  writes an unattended COO may make (start, send to and stop chats, tasks,
  notes in the vault, workspaces, a notification, a learning proposal, a
  meeting prep note). Every Confirm and External action is refused at once in
  every permission mode, with no card, and so are the `tracker.*` write
  actions, anything that needs Hollow's window, the login Keychain, Finder or a
  browser sign-in, and the actions outside that first set. The refusal tells
  the agent to put the request in its reply; `.app-actions.jsonl` in the vault
  records each one. The full list:
  [Core startup and `hollow serve`](reference/architecture.md#core-startup-and-hollow-serve).
  Claude Code's own permission prompts are refused too. M2 routes cards to the
  person's Mac.
- Linear still gets writes from serve another way: with "Track agent work" on,
  a chat that serve starts or sends to queues its Linear issue and its Started
  and Outcome comments in the work outbox, and serve's tracker sync sends
  them (with the Linear key in serve's secrets file). Only the `tracker.*`
  tool calls are refused.
- Limits: a Hollow app tool call has no timeout (as in the app), and nothing
  isolates a handler's panic. The release build aborts on panic, so a panic in
  any tool call serve runs takes the whole daemon down; launchd restarts it
  and the runs it left are settled at the next boot sweep.
- The schedule window jobs (weekly update, Friday status, gardener, as-of
  snapshot, friction) do not start; serve logs that at start. COO check-ins
  do not run in serve (see The runner COO).
- Adding a Claude account, the move-home rollback guard, learning capture.

## Rollback

```sh
sudo launchctl bootout system/com.hollow.app.serve     # SIGTERM, settles, exits
sudo rm /Library/LaunchDaemons/com.hollow.app.serve.plist
```

That is all launchd holds. `~runner/hollow-serve/` (binary, config, data,
`secrets.json`) stays until someone deletes it; delete `data/secrets.json` (or
`secret-delete` each key) and revoke the tokens at their providers when retiring
it. A paired device's token dies with `data/Support/Companion/phone.json`
(`$H serve disable-phone $D` turns the API off).
