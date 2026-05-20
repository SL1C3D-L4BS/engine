# ADR-025 · Audited crypto crates are not owned

- Status: Accepted
- Date: 2026-05-19
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)

## Context

The engine's guiding principle is to own its layers (R-02): owning a layer
means owning its behaviour, its bugs, and its determinism. The foundation
layer needs cryptographic primitives:

- **BLAKE3** — keyed hashing for the deterministic RNG (`engine-core`,
  spec IV.2) and for world-snapshot digests.
- **SHA-256** — content addressing for the asset pipeline (`engine-asset`,
  spec IV.8).
- **Ed25519** — signing and verifying Live Ops update paks (`engine-asset`).

## Decision

Cryptography is the one layer the engine deliberately does **not** own. The
foundation uses the audited, widely-deployed crates `blake3`, `sha2` (RustCrypto),
and `ed25519-dalek` / `curve25519-dalek`.

## Rationale

R-02 says owning every layer means owning every bug — and for cryptography
that is precisely the argument *against* owning it. A subtle bug in an owned
SHA-256 is a silent content-addressing corruption; a subtle bug in an owned
Ed25519 is a signature-forgery vulnerability. These crates are constant-time,
test-vector-verified, fuzzed, and audited by a far larger community than this
project can field. Re-implementing them would add risk, not remove it.

The determinism concern that motivates owning `engine-math` does not apply
here: cryptographic hashes are defined over exact integer operations and are
bit-reproducible by construction, on every platform, with no floating point
involved.

All three crates are permissively licensed (Apache-2.0 / MIT / BSD-3-Clause)
and pass the existing `deny.toml` policy.

## Consequences

- `engine-core` depends on `blake3`; `engine-asset` depends on `sha2` and
  `ed25519-dalek`. These are the foundation layer's only cryptographic
  dependencies.
- The engine still owns the *use* of these primitives — the RNG keying scheme,
  the content-address format, the pak signature envelope are all owned code.
  Only the primitive arithmetic is delegated.
- This ADR is the standing exception to R-02. Any future "own this layer"
  decision that touches cryptography is measured against it.
