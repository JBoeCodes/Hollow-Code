Fixtures for [../../phone-api/approvals.md](../../phone-api/approvals.md), identical in Hollow and `hollow-companion`.
`list.json`, `challenge.json`, `enroll-response.json` and the three `error-*.json` are exact: Hollow's tests
serialize its response types and compare. `enroll-request.json` is the shape only (a real attestation is about 5 KB, the
keys and proof are made up). Signatures from a real iPhone join these on the first device run (design, "Fixtures").
`attestation-vector.json` is an attestation object in the shape Apple's App Attest makes, built with openssl and
python (not by Apple, private keys discarded), with its root, message, key id and a fixed clock: Hollow's tests
verify it against that root and flip every byte. Replace or extend it with a capture from a real iPhone on the first
device run.

PR 2 adds `decision-challenge.json`, `decision-request.json` (shape only: the signature is made up),
`decision-response.json` and the five refusals (`error-approval-key-required.json`, `error-approval-expired.json`,
`error-approval-closed.json`, `error-approval-changed.json`, `error-bad-signature.json`), all exact except the request.

`decision-vector.json` holds two P-256 keys and two decision signatures made with Apple's Security framework (a software key,
the same DER a Secure Enclave key returns) over the message of `approvals.md`. Hollow verifies them; the app's tests
verify its own encoding the same way.
