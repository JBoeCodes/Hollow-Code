# Build Log

Chronological log of substantive changes, newest first.

## 2026-11-21: Initial public release (Hollow Code)

- **What:** Prepared clean public repository for Hollow Code. Generalized hardcoded integrations (Microsoft 365 OAuth, Linear tracker team/org pins, Notion importer defaults, updates host, bundle identifier, and signing team), removed internal benchmark and historical archive folders, and verified all unit, script, build, and Rust gates.
- **Why:** Publish Hollow Code as a clean, self-contained open-source desktop workspace with no private context, vault data, or organization-specific identifiers.
- **Verified:** `npm run gate` (frontend typecheck, vitest, script tests, production build, bundle budget, `cargo fmt --check`, `cargo clippy --lib --tests -- -D warnings`, `cargo test --lib`) and zero-leak audit scan.
