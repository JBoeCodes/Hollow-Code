# Computer control: how agents see and act on a desktop

Computer control lets an agent in a chat look at a screen and click and type, through one
Hollow-owned path. The design and the reasons are in the build workspace's
`outputs/cua-computer-use-design-2026-11-15.md`; this file is what the app does now. The
build narrative is in [BUILD_LOG.md](BUILD_LOG.md) (rows CU1 to CU9); the agent-facing
details (every rule, with its test) are in [reference/agent-tools.md](reference/agent-tools.md)
under "Computer control (Cua)".

## The pieces

| Piece | File | What it is |
|---|---|---|
| Basic verbs | `src-tauri/src/computer_mcp.rs` | `open_url`, `open_app`, `list_apps`, `search_bookmarks`, `browser_current_tab`, `browser_list_tabs`, `quit_app`. Always there when Computer control is on. Tier 2 verbs ask first outside Bypass. |
| Broker | `computer_broker.rs` | The one place a see-and-act call reaches the engine (Cua Driver, MIT, pinned by version and SHA-256 in `computer-engine.json`). Decides the cards, enforces the bounds, writes the audit row. |
| This Mac's policy | `computer_thismac.rs` | Which app a call names, the default deny list, the bounded manifest. |
| Shell path | `computer_cli.rs` | `hollow computer tools` and `hollow computer call <tool> '<json>'`: the same code and the same broker, for an agent without an MCP flag. |
| Settings | `src/settings/ComputerControlSheet.tsx`, `computer_commands.rs`, `computer_doctor.rs` | Settings, Connectors, Built into Hollow, Computer control, **Set up**: the switches, the apps agents can't use on This Mac, Stop all, and the Doctor's checks with a copyable fix each. |
| In the chat | `computer_activity.rs`, `ComputerChip.tsx`, `ComputerCard.tsx` | A Desktop chip in the composer and a Computer section in the side panel, once the chat has used the desktop: the latest frame, the recent actions with their tier, Pause, Take over, Hand back and Stop. The tray menu has **Stop all computer control**. |

Nothing is served until a person puts `computer-engine.json` in Hollow's support folder: no
engine ships, and a bad pin serves nothing.

## What an agent can do, by tier

| Tier | What | Asks |
|---|---|---|
| Observe | screenshot, accessibility tree, window list | Per chat (see the table below) |
| Act | click, type, press keys, scroll, drag | Per chat |
| Reach | clipboard, files, closing another app | Each time, in every mode but Bypass |
| Always asks, every mode | A send, post, payment or delete; typing into a password field; the first use of This Mac's screen in a chat; a look at This Mac's whole screen | Each time |

A tool with no row in the tier map is never listed and is refused: the Driver ships new tools
nearly every release.

## What each permission mode does

| Mode | Sandbox Mac | This Mac's screen |
|---|---|---|
| Plan | Nothing attaches. | Nothing attaches. |
| Ask (default) | One card per chat, then observe and act run. | First-use card, then a card per app per chat. |
| Accept edits, Auto | No card for observe and act. | As Ask. |
| Bypass | No card; Reach runs. | First-use card; no per-app card. |

The outward guard (a click or key that sends, posts, pays or deletes) and the password-field
rule read only the call's own arguments, so they miss an unlabeled icon button. The sandbox is
the real boundary; the guard is the second line.

## This Mac's screen

Off until the person turns it on (Settings sheet). A call names exactly one app by bundle id
(or a name); the Driver is started per run with a manifest naming only the apps that run has
been allowed. A default deny list (terminals, System Settings, Keychain Access, password
managers, Cua Driver, Hollow itself) is refused in every mode, with no card, until the person
allows an app in the same sheet. The Driver app is installed by the person, who grants it
Accessibility and Screen Recording (to the Driver, not to Hollow).

## Stopping it

- **Stop on the chat** ends that run's session.
- **Computer control off** (Settings) refuses the next call everywhere and ends live sessions.
- **Stop all computer control** (Settings sheet): ends every session and forgets every grant,
  with no chat open.
- **Bounds:** a run ends after 30 minutes or 300 acts, or when the same call repeats three times
  or fails five times in a row. A This Mac session idle for 5 minutes ends on its own.

## The audit

Every call whose proof checks out leaves one `computer_action` row in the hash-chained audit
log: target, tool, tier, argument names, the element and app (cut), coordinates, typed length
(and a hash only from 16 characters), the SHA-256 of each screenshot, and the cards that let it
run. Typed text and screenshots are never stored.

## Not built yet, and not verified

- The sandbox Mac (a VM on the Mac mini), its health check and the golden image (CU5, CU11),
  the mini's append-only trail (CU12), and the spike that measures it (CU0) wait on a licence
  call, a consent click in the VM and a go from Alex. The steps, with what runs the moment he
  does each, are in the build run's `computer-needs-review.md`; the CU12 script is ready and
  needs no `sudo`.
- The Doctor lists what it cannot check yet (the Driver's own permissions and telemetry, Lume, the
  VM volume) as "Not checked".
- **Untested by hand:** no real Cua Driver has run against any of this. The argument names that
  carry the app, the manifest's field names and `--manifest` flag, and the Driver's real tool
  names for sessions and the clipboard are assumptions, each in one place in the code, until the
  Driver is installed and CU0 reads its schema. The chat chip and card were driven only against a
  mock desktop.
- Whether Grok's and Antigravity's sandboxes can reach the broker's socket is unverified (it
  takes a paid test turn); until it is, Hollow attaches nothing to those agents and the model
  picker says "Can't use the desktop in this chat".
