# Approvals from the phone: enrollment, reading and answering (v1.2, build run 11-17, row B4, ENG-172)

Contract for the routes of [approvals-design.md](approvals-design.md) (PR 1: enrollment and reading; PR 2: answering and
the broker rules); fixtures in `docs/phone-api-fixtures/approvals/`. The design is the reasoning (threat model, the two keys, why App Attest); this
file is what is on the wire. Every base64url value is
without padding. All routes need the phone's bearer token (scope `phone`: another Mac's token gets 403).

## What reading and enrolling mean

Reading the pending cards needs no key. Answering one needs the keys below. Turning approvals on needs the Mac: the person
opens an approvals code window for this phone on the Mac (8 characters from the pairing alphabet, 10 minutes, one at a
time, in memory, closed by a successful enrollment or five wrong proofs). A bearer token alone enrolls nothing.

## `GET /v1/approvals`

200 `{"enrolled": bool, "items": [Approval]}` (fixture `list.json`), oldest first. `enrolled` is whether this phone has
approval keys. An `Approval`:

| Field | |
|---|---|
| `id` | the card's id, `apr-<launch>-<n>`: an opaque string, unique across Hollow's restarts |
| `sessionKey`, `chatTitle` | the chat that asked, or `null` for a card no chat owns |
| `title`, `action` | the card's text as the Mac shows it |
| `kind` | `app`, `app-external`, `computer` or `mail` |
| `createdAt`, `expiresAt` | RFC 3339; when the wait ends unanswered |
| `digest` | SHA-256, 64 lower-case hex digits, of `title + "\n\n" + action` (UTF-8) |

An older Hollow answers 404 for the route: the app hides the setting and says to update Hollow on the Mac.

`GET /v1/waiting` gains items with `kind: "approval"`, one per card, before the chat question items: `questionId` is the
card id, `preview` the card title, `sessionKey` empty when no chat owns the card. An older app skips the kind.

The event stream sends `approvals` (data `{}`) to phones when a card appears or resolves; the app reads the list again.
The hello's `events` list includes it. A new push event `approval` ("<chat title>" and the card title) goes only to a
phone that is enrolled and registered for it (`POST /v1/push` with no `events` registers every event Hollow sends).

## Enrollment

1. `POST /v1/approvals/key/challenge` (empty body or `{}`) → 200 `{"challenge": "<32 random bytes>", "expiresAt"}`
   (fixture `challenge.json`; 60 seconds, one use; a new one replaces the open one). 403 `approvals_closed` when no
   window is open for this phone (fixture `error-approvals-closed.json`).
2. The phone builds the message, UTF-8, lines joined by `\n`, no trailing newline:

   ```
   hollow-approval-key-v1
   <deviceId>
   <approveKey>
   <denyKey>
   <challenge>
   ```

   with the keys as they are sent in step 3, attests an App Attest key over `SHA-256(message)`
   (`attestKey(keyId, clientDataHash:)`), and computes `proof` = lower-case hex of HMAC-SHA256 over the message keyed by
   the code's 8 characters (upper case, no dash or space).
3. `POST /v1/approvals/key` with `{approveKey, denyKey, challenge, keyId, attestation, proof}` (shape in
   `enroll-request.json`): the keys are base64url of their X9.63 uncompressed P-256 points (65 bytes), `keyId` is the
   standard base64 `generateKey` returned (32 bytes), `attestation` the base64url CBOR object.
   200 `{"fingerprint": "a1b2 c3d4", "enrolledAt"}` (fixture `enroll-response.json`); the fingerprint is the first 8
   hex digits of SHA-256(approve point followed by deny point), the same on both screens.

Hollow checks in this order, and stops at the first failure: the window is open for this phone (403
`approvals_closed`); the challenge is this phone's open one, unused and unexpired (409 `challenge_expired`, fixture
`error-challenge-expired.json`; the attempt uses the challenge up whatever follows); the proof, constant time (403
`wrong_code` with `attemptsLeft`, five close the window); both keys are on the P-256 curve and differ (400
`bad_request`); the attestation (400 `bad_attestation`, fixture `error-bad-attestation.json`, with a fixed reason
that names no secret). A refused attestation counts toward the window's five like a wrong proof (so the code's holder can't probe attestations for ten minutes). The keys must be canonical base64url (no padding): a key is compared by its text later.

The attestation follows Apple's "Validating apps that connect to your server": the `x5c` chain (credential certificate
and intermediate, nothing else) verifies to Apple's App Attestation root CA, pinned in Hollow
(`src-tauri/src/phone_access/apple_app_attestation_root.der`, SHA-256
`1cb9823ba28ba6ad2d33a006941de2ae4f513ef1d4e831b9f7e0fa7b6242c932`, from
`https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem`); the credential certificate's
extension `1.2.840.113635.100.8.2` holds SHA-256(authData followed by SHA-256(message)); SHA-256 of its public key is
`keyId`; the RP ID hash is SHA-256 of `TEAM000000.com.hollow.app.companion`, the counter 0, the AAGUID the production
one (`appattest` and seven zero bytes) and the credential id the `keyId`; when the authenticator data carries
`apple_validation_category_01` it is 2 (TestFlight), 4 (App Store) or 5 (enterprise or ad hoc), else refused (one
line to drop 5, design decision 8). An attestation with no extensions (iOS before 27) passes on the other checks.
Each link of the chain is verified with the digest its own signature algorithm names (ECDSA with SHA-256 or SHA-384, the same in and outside `tbsCertificate`), against a P-384 issuer key: Apple's credential certificate is signed with SHA-256 by the intermediate's P-384 key, the intermediate with SHA-384 by the root. The intermediate and credential certificates must be valid now, give or take an hour. The receipt is not used.

## `DELETE /v1/approvals/key`

This phone turns approvals off: 204, and its keys are gone. Turning off twice is fine. It needs only the bearer token, on purpose: dropping the keys only removes a capability (cards then wait for the Mac's window), never grants one.

## Answering a card

1. `POST /v1/approvals/{id}/challenge` (empty body) → 200 `{"nonce": "<32 random bytes>", "expiresAt"}` (fixture
   `decision-challenge.json`; 60 seconds, one use, at most 16 open per phone, the oldest dropped). 403
   `approval_key_required` (no keys), 409 `approval_closed` (the card is no longer pending).
2. The phone computes the digest itself from the text it shows (never the `digest` it was sent) and builds the
   message, UTF-8, lines joined by `\n`, no trailing newline:

   ```
   hollow-approval-v1
   <deviceId>
   <approval id>
   <nonce>
   <digest>
   <decision>
   <expiresAt>
   ```

   `<decision>` is `allow` or `deny`; `<expiresAt>` is the challenge's text exactly as sent. **Allow** is signed with the
   approve key (Face ID on every use), **Deny** with the deny key (no prompt): `SecKeyCreateSignature(key,
   .ecdsaSignatureMessageX962SHA256, message)`, a DER (X9.62) signature.
3. `POST /v1/approvals/{id}/decision` with `{decision, nonce, digest, signature}` (fixture `decision-request.json`;
   the signature is base64url of the DER) → 200 `{"resolved": "allowed" | "denied"}` (fixture
   `decision-response.json`).

Hollow checks in this order and stops at the first failure: the phone has keys (403 `approval_key_required`); the nonce
is an open challenge of this phone's for this card (409 `approval_expired`); the card is still pending (409
`approval_closed`); the sent digest is the card's current one (409 `approval_changed`: read the card again and sign the
new text, the same challenge still stands); then the signature over the message Hollow rebuilds from its own copy of the
challenge, with the approve key for `allow` and the deny key for `deny` (400 `bad_signature`; fixtures
`error-*.json`). The nonce is used up before the card resolves. A bad signature burns its challenge; five within ten
minutes from one phone drop its keys, and the Mac's row says so. A changed card, an expired challenge or a closed card
counts for nothing. The first answer wins, from the window or any phone: the others get 409 `approval_closed`.

The record: the card's tool badge reads "Allowed on iPhone" or "Denied on iPhone", and the audit line
(`.app-actions.jsonl`, `cardBy`) and the `approval-resolved` event (`answeredBy`) carry the receipt ("Allowed on
Alex's iPhone with Face ID at 7:42 AM").

Two limits, stated plainly: the five-strike rule means a holder of a copied bearer token can make the real phone lose its
keys (a denial of service, never an approval; the person turns approvals on again at the Mac), and "with Face ID" in
a receipt is what the key's policy guarantees (the approve key signs only after Face ID), not something Hollow can
observe.

## When a card waits for a phone

While any phone has approvals on: a hollow_app card waits even when the Mac's window is closed to the tray (it is
shown when the window opens, and pushed to the phone); every card waits 5 minutes (a setting, 2 to 15 minutes, in the
phone's row in Settings) instead of 120 seconds, which holds an unanswered turn up to 3 minutes longer. For computer and
mail cards the timer lives in the MCP client: a client that sends `"waitFromBroker": true` gets a first line
`{"waitSecs": N}` as soon as the card shows and waits N seconds (a first line that is a verdict is the verdict; an older
client sends no such field and keeps its 120 seconds; an older broker sends no such line). Without an approval phone
nothing changes.

## Stored

`Device.approvalKeys` in `phone.json`: `{approveKey, denyKey, fingerprint, enrolledAt}`; absent for a phone without
approvals and in a file an older Hollow wrote.
