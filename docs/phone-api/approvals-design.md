# Approvals from the phone, behind Face ID (design; Hollow side built, see approvals.md)

Status: **design; the Hollow side is built (B4 PR 1 and PR 2, 2026-11-19: enrollment, the read routes, the waiting kind, the event and the push, challenge and decision, the broker's longer wait and the Mac's Settings row).** The iPhone app (PR 3) follows. It is item 14 of the companion app's roadmap (Phase 4,
"Approvals from the phone (installs, confirm cards) behind Face ID. Last, because it acts on the Mac and needs its own
security design"), written by overnight lane I1 on 2026-11-07. It is identical in Hollow (`example-org/hollow-code`) and
hollow-companion (`example-user/hollow-companion`); change both at once. The contract for the routes below gets its own
file (`docs/phone-api/approvals.md`) and fixtures when the work starts; this file is what that contract builds to.

## What it is for

Hollow asks the person before some actions run: an Allow/Deny card in the chat. Today the card can only be answered in
the Mac's window, so an agent that needs one while the person is away waits two minutes, gets "no answer", and moves
on; with the window closed to the tray, a hollow_app action's card is refused at once.
This design lets the person approve or deny those cards from the iPhone app, with Face ID proving that the person, not
just the phone, approved.

In scope: every card Hollow's approval broker shows (`approvals.rs`):

| Card kind | What asks | Examples |
|---|---|---|
| `app` | a hollow_app action of tier confirm | settings changes in some modes, `sessions.start` in auto |
| `app-external` | a hollow_app action of tier external (its card names a destination) | `build.publish` (a push or pull request) |
| `computer` | computer control (`computer_mcp.rs`) | opening or quitting an app |
| `mail` | an approved mail connector's tool that isn't a read (`remote_mcp.rs`) | a connector send |

The built-in Microsoft 365 connector (`m365_mcp.rs`) is read-only and never asks, so it has no cards.

The always-ask actions (`app_actions::ALWAYS_ASK`: `build.install_built_app`, `gardener.apply`,
`records.records_supersede_decision`, `records.records_set_decision_status`, `friction.draft_fix`, `settings.cfo_approve_extra_usage`;
`tracker.archive_task` left the list on 2026-11-15, T1) are
the main reason for this work: installs ask in every mode, bypass included.

Out of scope, and why:

- **An agent's own permission prompts.** Hollow has none to forward: Claude and Grok run headless with
  `--permission-mode`, and Hollow declines every Codex approval request (`codex_app_server.rs`, `NO_APPROVALS`).
- **Question cards.** Answered from the phone since v1.1 (`POST /v1/sessions/{key}/questions/{id}/answer`). They
  need no Face ID: an answer is a message, like a send.
- **The Review inbox, Results acceptance, learnings and proposals.** Person-only decisions with their own flows and
  fingerprints. They can reuse the keys below later; they are not part of this design.

## How approvals work today (Hollow, read 2026-11-07)

- **Asking.** `app_broker.rs` decides whether an action asks (`asks_in_mode`; the always-ask list in every mode) and
  calls `AppAsker::ask`, which goes to `approvals::ask_app` and `ask_in_process`, in Hollow's own process. Computer
  control (kind `computer`) and an approved mail connector's writes (kind `mail`) ask from their MCP subprocess through
  `approvals::confirm_tool`, over the `HOLLOW_APPROVAL_SOCK` socket (`request_via_app_at` on the client side,
  `serve_conn` in the broker).
- **Pending cards** live in memory only: `PENDING` (id to a one-shot sender) and `CARDS` (id to `PendingCard`:
  id, title, action text, kind, chat key, time). Ids are `apr-<n>` from a counter that starts at 1 when Hollow starts.
- **The window** gets an `approval-request` event (id, title, action, kind, tool, toolUseId, sessionKey) and shows
  the card in that chat (`chatApprovals.ts`, `ChatBody.tsx`). Allow or Deny calls the IPC command
  `approval_respond(id, approved)`, which calls `approvals::respond`. `approval-resolved` carries the reason
  (`allowed`, `denied`, `timed_out`, `cancelled`).
- **Timeout:** 120 seconds (`CLIENT_TIMEOUT_SECS`), kept in two places. An app card waits `ASK_TIMEOUT` in
  `ask_in_process`. A computer or mail card waits in its MCP client: `request_via_app_at` gives up after 120 seconds on
  its own clock and writes `{"cancel":"timed_out"}`, and the broker's `serve_conn` waits on that client. Only an
  answered Allow counts as allowed; a Deny reaches the agent as "the user declined", a different outcome from a
  timeout. The run's no-output watchdog is held while a card waits.
- **Window closed:** the app broker refuses its cards at once, with no card, when the main window is closed to the
  tray (`Asker::can_show`, S15). Computer and mail cards still open in the closed window and wait out their 120
  seconds.
- **The install card** (`build.install_built_app`) names the commit, the diff stat against the installed build and up
  to 12 changed files ("and N more"), not the diff itself.
- **The phone API** has no approvals: `GET /v1/waiting` lists `question` items only and reserves approvals as a later
  kind behind Face ID (`docs/PHONE_CHAT_API.md`); push sends `turnFinished` and `question`; unknown kinds and events
  are skipped by design.
- **Phone credentials:** a bearer token (32 random bytes; Hollow keeps only its SHA-256) per paired phone, plus the
  Host and Tailscale login checks. `Device` (`phone_access/store.rs`) has an id (`ph-` and 8 hex digits), name,
  model, token hash and push registration, in `<app support>/Companion/phone.json` (0600). Revoke removes it. Pairing
  has one window at a time, in memory, for whichever phone types its code first; a successful pairing closes it.
- **Signing that already exists:** the homelab companion inbox verifies device ops with ECDSA P-256 over exact payload
  bytes, with an applied-op ledger keyed by device and op id, and proves a device's public key at pairing with an HMAC
  of the pairing code (`companion_inbox::pair_proof`). This design reuses both patterns. One difference: the inbox
  verifies raw `r‖s` signatures (`ECDSA_P256_SHA256_FIXED`), while the phone's Secure Enclave signs in DER (X9.62), so
  this design verifies with `ECDSA_P256_SHA256_ASN1`.

## What Face ID has to guarantee (threat model)

1. **A stolen bearer token approves or denies nothing.** The token reads chats and sends messages; it can leak through
   a device backup, a log or a compromised app container. Answering a card needs a key that never leaves the phone's
   Secure Enclave.
2. **An unlocked phone in someone else's hand approves nothing.** The approve key can only sign after Face ID, each
   time, with no reuse window.
3. **A signed answer can't be replayed or moved.** A signature covers one card, the exact text the phone showed, a
   one-time nonce from Hollow, the decision and an expiry. An old signature, one for another card, or one for a card
   whose text changed is refused.
4. **Enrolling a key needs the Mac.** Whoever holds the bearer token must not be able to enroll their own key, so
   turning approvals on takes a one-time code shown on the Mac (the same proof as pairing).
5. **Only the genuine app enrolls a key.** A modified build of the app could create an ordinary key that Face ID
   doesn't guard and enroll it with the Mac's code. iOS can't attest a Secure Enclave key's own flags, so the phone
   proves the app instead: an App Attest attestation over the enrollment (below). Only an unmodified build of this app,
   signed by this team, on a real iPhone gets one, and that app only ever makes the approve key in the Secure Enclave
   with `.biometryCurrentSet`.
6. **A new face doesn't inherit the right.** The approve key is bound to the current biometric enrollment: adding a
   face or finger to the phone invalidates it, and the person turns approvals on again.

Not defended, and stated plainly in the app: a person tricked into approving (the phone shows exactly what the Mac
would, and can't judge intent), a compromised Mac (Hollow is the party asking), and a jailbroken phone that tampers
with the genuine app after App Attest vouched for it.

Why not just call `LAContext.evaluatePolicy` and then send "approved": Hollow can't tell whether Face ID happened. A
modified app or a stolen token skips it. A Secure Enclave key whose use requires biometry makes the proof
cryptographic, and App Attest at enrollment makes it the genuine app's key.

## The design

### 1. Two keys per phone, enrolled once

- **On the phone:** Settings → Approvals → "Approve from this iPhone" (off by default). Turning it on creates two P-256
  keys in the Secure Enclave (`kSecAttrTokenIDSecureEnclave`, `kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly`):
  - the **approve key**, access control `[.privateKeyUsage, .biometryCurrentSet]`: Face ID on every use;
  - the **deny key**, access control `[.privateKeyUsage]`: no prompt, but like the approve key it never leaves this
    phone, so a copied bearer token can't deny either.

  Neither is exported, backed up or moved to another phone.
- **On the Mac:** Settings → Devices → Companion devices → the phone's row → "Approvals from this iPhone: Turn
  on". That opens an approvals code window for that one phone: an 8-character code from the pairing alphabet for 10
  minutes. It is separate from the pairing window and works like it: in memory, one at a time, closed by a successful
  enrollment (one use), by its 10 minutes or by five wrong proofs.
- **Enrolling** takes two calls:
  1. `POST /v1/approvals/key/challenge` → `{ "challenge": "<32 random bytes, base64url>", "expiresAt" }` (60 seconds,
     one use).
  2. The phone builds the enrollment message (UTF-8, lines joined by `\n`, no newline at the end):

     ```
     hollow-approval-key-v1
     <deviceId>
     <approveKey>
     <denyKey>
     <challenge>
     ```

     makes an App Attest key (`DCAppAttestService.generateKey`) and attests it with
     `attestKey(keyId, clientDataHash: SHA-256(message))` (this needs the phone to reach Apple once), then sends
     `POST /v1/approvals/key` with `{ "approveKey", "denyKey", "challenge", "keyId", "attestation": "<base64url>",
     "proof": "<hex>" }`. Each key is base64url of its X9.63 uncompressed point (65 bytes); `keyId` is the string
     `generateKey` returned (standard base64 of 32 bytes); `proof` is lower-case hex of HMAC-SHA256 over the same
     message, keyed by the code's 8 characters as Hollow normalizes a typed code (upper case, no dash or space:
     `phone_access::store::normalize_code`), not the dashed form the Mac displays.
  3. Hollow checks, in order: the approvals code window is open for this device and the challenge is this phone's,
     unused and unexpired; the proof (constant-time; a wrong one counts toward the window's five); both keys are valid
     P-256 points; then the attestation, following Apple's "Validating apps that connect to your server":
     - the `x5c` chain (the credential certificate first, then the intermediate) verifies to Apple's App Attestation
       root CA, pinned in Hollow, each signature with the algorithm its certificate names (Apple's App Attestation CA
       keys are P-384: `ring`'s `ECDSA_P384_SHA384_ASN1`, not the phone keys' P-256 verifier);
     - the credential certificate's extension `1.2.840.113635.100.8.2` is a DER sequence holding a single octet
       string, and that octet string equals SHA-256(authData ‖ SHA-256(message));
     - SHA-256 of the credential certificate's public key (X9.62 uncompressed) equals `keyId` decoded from base64;
     - the authenticator data's RP ID hash equals SHA-256 of the App ID `TEAM000000.com.hollow.app.companion`, its
       counter is 0, its AAGUID is the production one (`appattest` and seven zero bytes: TestFlight and the App Store
       use production), and its credential id equals the decoded `keyId`;
     - the `apple_validation_category_01` value (a UInt32) in the authenticator data's `extensions` CBOR dictionary
       is 2 (TestFlight), 4 (App Store) or 5 (enterprise or ad hoc) when it is there; any other value is refused.
       iOS adds the extensions only from iOS 27 and the app supports iOS 18, so an attestation without them passes
       on the other checks (it can't be stripped: the nonce binds the authenticator data).

     The production AAGUID alone doesn't tell those three apart, so the category is read explicitly. Category 5 is
     accepted on purpose: only Alex's developer account can sign a build of this App ID, so the design treats a
     same-team ad hoc build as his. Requiring 2 or 4 only is a one-line change if he wants it (decision 8 below).
     Apple's step list, checked 2026-11-07, has these categories; check it again when the work starts.

     Then Hollow stores `approvalKeys: { approveKey, denyKey, fingerprint, enrolledAt }` on the `Device` record and
     closes the code window. Both sides show the fingerprint, the first 8 lower-case hex digits of SHA-256(the approve
     key's 65 bytes followed by the deny key's 65 bytes), as two groups of four, so the person can compare.

  App Attest isn't available in the Simulator: the mock server accepts a test attestation for the UI tests; Hollow has
  no such path.
- **Turning off:** either side. The phone deletes both keys and calls `DELETE /v1/approvals/key`; the Mac's Turn off
  drops the stored keys. Revoking the phone drops them with everything else.

### 2. Pending cards on the phone (reading)

- `GET /v1/approvals` → `{ "items": [ { "id", "sessionKey", "chatTitle", "title", "action", "kind", "createdAt",
  "expiresAt", "digest" } ] }`, oldest first: the pending cards this phone may answer, for every phone (reading needs
  no key; answering does). `digest` is the SHA-256 (64 lower-case hex digits) of the card's canonical text:
  `title + "\n\n" + action`, UTF-8, as the card shows it.
- `GET /v1/waiting` gains `kind: "approval"` items (one per card, before that chat's question item), so the "Waiting on
  you" list shows them. Older apps skip the kind, as the contract already says.
- The event stream sends `approvals` when a card appears or resolves; the app reads the list again.
- Push: a new event name, `approval` ("<chat title>: <card title>"), registered like the others. It opens the app; no
  notification action approves (a notification action can't hold the Face ID prompt the approve key needs).
- **Card ids must be unique across restarts:** `apr-<n>` restarts at 1, so an id becomes `apr-<boot>-<n>`, where
  `<boot>` is 8 random hex digits chosen when Hollow starts. The window treats the id as an opaque string
  (`chatApprovals.ts`), so nothing there changes.

### 3. Approving and denying (writing)

1. The phone shows the card: chat, title, the whole action text (for an install, the commit, diff stat and changed
   files the Mac's card names), and the time left. Buttons: **Deny** and **Approve**.
2. Either button asks Hollow for a challenge: `POST /v1/approvals/{id}/challenge` → `{ "nonce": "<32 random bytes,
   base64url>", "expiresAt": "<now + 60 s>" }`. Hollow keeps at most 16 open challenges per phone, each usable once,
   with the card id, the device id and the expiry it handed out.
3. The phone computes the digest itself from the text it shows (never the server's `digest` field) and builds the
   message (UTF-8, lines joined by `\n`, no newline at the end; `<nonce>` and `<expiresAt>` exactly as the challenge
   sent them, `expiresAt` being RFC 3339 UTC with whole seconds and `Z`):

   ```
   hollow-approval-v1
   <deviceId>
   <approval id>
   <nonce>
   <digest>
   <decision>
   <expiresAt>
   ```

   where `<decision>` is the one word `allow` or the one word `deny`. **Approve** signs it with the approve key, **Deny** with the deny key: `SecKeyCreateSignature(key,
   .ecdsaSignatureMessageX962SHA256, message)`. The approve key's reference is read with
   `kSecUseAuthenticationContext` set to an `LAContext` whose `localizedReason` names the card ("Approve: <card
   title>"); Face ID shows when it signs. The system may not show that reason during a Face ID scan, so the app keeps
   the card's whole text on screen behind the sheet.
4. The phone sends `POST /v1/approvals/{id}/decision` with `{ "decision": "allow" | "deny", "nonce", "digest",
   "signature": "<base64url DER>" }`. Hollow checks, in order: the phone has approval keys (else `403
   approval_key_required`); the nonce is an open challenge of this phone's for this card, unused and unexpired (else
   `409 approval_expired`); the card is still pending (else `409 approval_closed`); the sent `digest` equals the card's
   current digest (else `409 approval_changed`: the phone reads the card again and shows the new text). Then it
   rebuilds the message from its own copy of the challenge (device id, card id, nonce, expiry) with the sent digest and
   decision, and verifies the signature with the approve key for `allow` or the deny key for `deny`, using
   `ECDSA_P256_SHA256_ASN1` (else `400 bad_signature`). It marks the nonce used before resolving, then calls
   `respond_from(id, allowed, by: this device)`. `200 { "resolved": "allowed" }` or `{ "resolved": "denied" }`.
5. The first answer wins, from the window or any phone; the others get `409 approval_closed` and the card shows who
   answered. Only `bad_signature` counts toward the limit: five from one phone within 10 minutes drop its approval
   keys, and the Mac says so in the Companion devices row. A changed card, an expired challenge or a closed card is
   not a bad signature and counts for nothing.
6. **The record.** The card's receipt and History say where it was answered: "Allowed on Alex's iPhone with Face
   ID at 7:42 AM", "Denied on Alex's iPhone at 7:43 AM". `Verdict` gains who answered (the window or a device id).

Every base64url value in these routes is without padding.

### 4. When a card waits for the phone

Two broker rules change once a phone has approvals on:

- **Window closed to the tray:** today the app broker refuses its cards at once (S15), while computer and mail cards
  open in the closed window and wait. With an approval phone, app cards wait too and every card is pushed to the
  phone; the window shows them when it opens.
- **Timeout:** 120 seconds is short for a push, an unlock and Face ID while away. With an approval phone every card
  waits 5 minutes (a setting beside the toggle), and the run's watchdog stays held as it is today. The broker uses
  the setting for app cards (`ask_in_process`). For computer and mail cards the timer is in the MCP client, so the
  broker tells the client: a client that sends `"waitFromBroker": true` in its request line gets a first line
  `{ "waitSecs": N }` back as soon as the card shows, and waits N seconds instead of its own 120; when N runs out it
  writes `{"cancel":"timed_out"}` as today. A client that doesn't ask (a turn started before an update) gets no such
  line and keeps its 120 seconds, so an old client never reads the new line as a verdict. A new client keeps its own
  120 seconds until a wait line arrives (an older broker sends none), and a first line that is already a verdict
  (`approved` or `refused`) is the verdict, as today. `serve_conn` has no timer of its own and gets none: the client
  stays the one clock for these cards.

Without an approval phone nothing changes.

### 5. Older Hollow, older app

- The app reads `GET /v1/approvals`: a 404 hides the Approvals setting and says "Update Hollow on your Mac to approve
  from the phone". Cards stay Mac-only.
- An older app ignores the `approval` waiting kind, the `approvals` event and the push event: it never sees a card it
  can't answer. A card it doesn't show waits for the window, as today.

### 6. Where each control lives (one home each)

| Control | Where |
|---|---|
| Turn approvals on or off for a phone (Mac) | Settings → Devices → Companion devices → the phone's row |
| Turn approvals on or off (phone) | the app's Settings → Approvals |
| See and answer a card (phone) | the card inline in its chat, and the Waiting on you list |
| See who answered | the card's receipt and History |

## What gets built (when it is)

**Hollow** (`src-tauri/src`):

- `approvals.rs`: ids unique across restarts; `PendingCard` keeps `digest` and `expires_at`; a read of pending cards
  for the phone; `respond_from(id, allowed, by)`, which takes the card's one-shot sender under the same lock `respond`
  uses today and records `by` there, so the first answer wins atomically (the window's `approval_respond` calls it with
  `by = Window`, so the Mac's Allow and Deny work as before); `serve_conn` answers `waitFromBroker` with the wait line,
  and `request_via_app_at` asks for it and uses it.
- `app_broker.rs`: `can_show` is also true while a phone has approvals on; the timeout from the setting, and
  `declined`'s "within two minutes" says the wait the card really had.
- `phone_access/store.rs`: `Device.approval_keys` (serde default, so older files read); the approvals code window,
  like `Pairing`.
- `phone_access/routes.rs`: the enrollment challenge and key routes, the list, challenge and decision routes; the
  waiting kind; the stream event. `phone_access/push.rs`: the `approval` event. Challenges in memory per phone (16 at
  most, 60 seconds).
- App Attest verification: a CBOR decoder for the attestation object, an X.509 parser for the chain and the nonce
  extension, and Apple's App Attestation root CA pinned in the binary (new dependencies; `ring` verifies the chain's
  P-384 signatures and the phone keys' P-256 ones).
- Settings → Devices → Companion devices: the Approvals row and its code.
- Tests: signature checks with fixed test keys (the fixtures), replay (a nonce used twice), an expired challenge, a
  changed card (`approval_changed`, not counted as a bad signature), a revoked phone, no keys, a deny signed with the
  approve key (refused), a deny without a signature (refused), first answer wins, five bad signatures drop the keys, an
  attestation for another app, with a wrong nonce or with a validation category other than 2, 4 or 5 (refused),
  one without the extensions, as iOS before 27 makes it (accepted), an older `phone.json` without the field, an old socket
  client that doesn't ask for the wait line.

**The app** (`ios/`):

- An `ApprovalKeys` type in the app target (Security, LocalAuthentication and DeviceCheck; `HollowChatKit` stays
  Foundation only): create both keys, attest, delete, sign.
- Settings → Approvals: Turn on (type the Mac's code), Turn off, the fingerprint, and why it's unavailable (no Face ID
  set up: no passcode fallback in this version; App Attest unsupported).
- The approval card in the transcript and the Waiting on you list; the push event; the mock server's routes with test
  keys and a test attestation; a UI test of approve and deny against the mock.

**Fixtures** (`docs/phone-api-fixtures/approvals/`, identical in both repos): P-256 key pairs for tests only, a message
and its signatures, the list, challenge and decision bodies, and each refusal. The signatures are made with Apple's
Security framework (`SecKeyCreateSignature` on a software P-256 key in the app's tests, the same DER the Secure Enclave
returns), not only with `ring`, so Hollow's tests verify the encoding the phone produces. One signature and one
attestation from a real iPhone are added on the first device run.

**Order:** PR 1 Hollow: enrollment with App Attest, the read routes, the waiting kind and push (nothing approves yet).
PR 2 Hollow: challenge, decision, the broker rules (including the socket clients' wait line), Settings. PR 3 app: keys,
attestation, Settings, card, Face ID, mock and tests. Each PR merges on its own; the Hollow side first.

## Decisions for Alex before it is built

1. A card waits for the phone when the Mac's window is closed to the tray (today an app card is refused at once).
   Proposed: yes, only while a phone has approvals on.
2. How long a card waits: proposed 5 minutes while a phone has approvals on (120 seconds otherwise, as today).
3. The longer wait also applies while the person is at the Mac: it ends the moment anyone answers, but an unanswered
   card holds the turn 3 minutes longer than today. Proposed: yes, for every card, rather than guessing whether the
   person is at the Mac.
4. Which cards the phone may answer: proposed every kind, the six always-ask actions included.
5. Deny without Face ID: proposed yes, signed by the phone's deny key (no prompt), so a copied bearer token still can't
   deny.
6. A phone without Face ID: proposed no approvals (no passcode fallback) in this version.
7. App Attest at enrollment: the phone reaches Apple once when approvals are turned on, and Hollow gains CBOR and
   certificate checks. Proposed: yes; without it, a modified build of the app could enroll a key that Face ID doesn't
   guard, and threat 5 above is not defended.
8. Which builds may enroll (Apple's validation category): proposed TestFlight, App Store and a same-team ad hoc or
   enterprise build (2, 4 and 5); TestFlight and App Store only is the stricter choice.
