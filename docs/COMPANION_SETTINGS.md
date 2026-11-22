# Companion settings

Settings → Devices → Companion devices (its own section until P1-22; the `companions` id still lands there) uses `src/settings/CompanionDevicesPanel.tsx`. The backend block starts with its
connection state, saved URL, task/document counts, scope and last successful task push. Missing document counts
stay "Not reported"; read failures show a retry and identify any retained status as last-good data.

The saved connection and scope selection use shared solid reading materials.
Counts use a wrapping definition grid with tabular figures; missing values keep
their words. Workspace names wrap fully beside stable checkboxes and status
words inside the existing bounded scroll list. These styles do not change
selection, confirmation, polling or backend traffic.

Connection state (B05): **Connected**; **Waiting for the backend** only before its first answer; **Can't reach the
backend** once the bridge has a `lastError`. That error shows once, under the address, with "Last reached <time>"
(`lastContactAt`, or "Not reached yet"); the "needs attention" details list only the other errors (mirror, documents,
scope), and drop one that repeats it word for word. A backend that gives no answer at all is described by
`unreachable_words` in `companion_bridge.rs`, the same words for the live status, **Test connection** and
**Connect**: a timeout names its seconds, anything else says Hollow couldn't reach the host and what to check. A
tailnet host (`*.ts.net`, or IPv4 in 100.64.0.0/10) adds "check that Tailscale is running and connected on this
Mac". A certificate or other secure-connection failure (rustls; reqwest counts it as a connect error, so
`Unreachable::of` reads the error's causes for "certificate", "tls", "ssl" or "handshake") says instead "Hollow
reached <host> but couldn't verify its secure connection (certificate). Check the backend's HTTPS certificate, or
the address.", plus "If the backend uses tailscale serve, check that its certificate has been issued." for a
`*.ts.net` name and no Tailscale-is-running line. An `https://` address in front of a plain-HTTP server (rustls:
"received corrupt message of type InvalidContentType", while connecting) says "Hollow reached <host> but it didn't
answer over HTTPS." and then, for this Mac (127.0.0.1, localhost, [::1]), to check whether the address should start
with http://, or elsewhere, since plain http is only allowed to this Mac, that the backend needs to serve HTTPS (for
example through tailscale serve or a reverse proxy) (R6). The badge reads **Can't reach the backend** only for the "Hollow
couldn't reach" and "didn't answer within N seconds" words; any other `lastError` (this one, a refused token, an
HTTP status) reads **Backend needs attention**. HTTP answers (401, other statuses) keep their own wording. **Turn off** that fails says which half failed:
nothing changed (the replies still owed to the backend couldn't be cleared), or the replies were cleared but the
settings file couldn't be deleted; either way Turn off can be chosen again.

An unconfigured backend opens Connect to the homelab. A configured backend folds the same form under Change
connection. Open setup page accepts HTTPS or loopback HTTP only, with no credentials/query/fragment. The page
provides a claim code only while no Hollow is connected. Pasting a password-masked applier token remains an
advanced fallback; no saved token is exposed. **Test connection** (ST5) checks the saved connection (beside Re-send all
tasks) or the typed URL and token (beside Save) without saving: is a companion backend there, and does it take the
token? The answer is one plain sentence under the button that asked; editing the URL or token, saving or re-sending
clears it. A good token claims or renews this Hollow's applier lease on that backend for 90 s, as connecting would. Claim/backend semantics stay in `docs/COMPANION_PROTOCOL.md` and
the companion repository's BACKEND_API.md.

Status polls run only while visible and idle. Each mutation invalidates older reads so a late status cannot undo
a completed turn-off. URL edits survive polls; credential controls cannot change during a save. Scope saves capture
the selection before confirmation and lock controls until completion. Failed saves retain the entered selection;
unknown scope never appears as zero workspaces. Existing work-data confirmation remains unchanged.

Tests: `tests/smoke/companion-settings.spec.ts` uses synthetic API replacements for connection state (including an
unreachable tailnet backend), delayed reads, credential changes and scope saves, plus keyboard-accessible disclosures and light/dark/glass accessibility/captures.
No live backend/pairing mutation or native install is part of these checks. Native VoiceOver/glass remain separate.
