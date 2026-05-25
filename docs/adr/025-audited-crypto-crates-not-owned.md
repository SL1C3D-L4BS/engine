# ADR-025 · Audited crypto crates are not owned

- Status: Accepted
- Date: 2026-05-19 (expanded 2026-05-24 per audit §15 ADR-025 sweep)
- Phase: 0 (pre-v1.0 — contract-exempt per risk R-03)
- Companion: ADR-008 (content-addressed asset pipeline — uses
  sha2 + ed25519-dalek), ADR-057 (owned BLAKE3 RNG — uses
  blake3), ADR-058 (cargo-geiger CI adoption — enumerates
  unsafe in these crates), ADR-052 (reproducible build cadence
  — locks the versions)

## Context

The engine's guiding principle is to own its layers (R-02): owning
a layer means owning its behaviour, its bugs, and its determinism.
The foundation layer needs cryptographic primitives:

- **BLAKE3** — keyed hashing for the deterministic RNG (`engine-core`,
  spec IV.2 / ADR-057) and for world-snapshot digests.
- **SHA-256** — content addressing for the asset pipeline (`engine-asset`,
  spec IV.8 / ADR-008).
- **Ed25519** — signing and verifying Live Ops update paks
  (`engine-asset`, also asset pak signatures per ADR-008).

## Decision

Cryptography is the one layer the engine deliberately does **not**
own. The foundation uses the audited, widely-deployed crates
`blake3`, `sha2` (RustCrypto), and `ed25519-dalek`.

### Version pins (as of audit close, 2026-05-24)

| crate            | pinned version | family                  |
| ---------------- | -------------- | ----------------------- |
| `blake3`         | `1.5.x`        | (BLAKE3 Team)           |
| `sha2`           | `0.10.x`       | RustCrypto              |
| `ed25519-dalek`  | `2.1.x`        | dalek-cryptography      |
| `curve25519-dalek` | `4.x` (transitive of ed25519-dalek) | dalek-cryptography |

The `Cargo.lock` file in the repo root is the authoritative pin.
This table is the explanation; lock file is the contract. The
reproducibility cadence (ADR-052) catches silent version drift.

### Security audits referenced

- **BLAKE3** — the reference C implementation has had multiple
  independent reviews; the `blake3` crate (Rust binding +
  pure-Rust portable path) is maintained by the BLAKE3 Team
  and used in production at major projects (cargo, b3sum,
  numerous Rust ecosystem tools).
- **`sha2` (RustCrypto)** — RustCrypto's `sha2` crate is the
  Rust ecosystem's de facto SHA-256 implementation; its test
  vectors include the NIST FIPS 180-4 corpus; widely audited
  through use in `cargo`, `rustup`, hundreds of crates.
- **`ed25519-dalek`** — the dalek-cryptography family is the
  Rust ecosystem's reference Ed25519 implementation; audited
  by Trail of Bits (multiple cycles); used by `age`, `ssh-key`,
  `ssh-agent`, large parts of the Rust crypto stack.

### Gate condition to revisit

A crate would be replaced (forked into the engine source tree,
upgraded, or replaced with another audited equivalent) if any
of the following triggers:

- A published CVE in the version family the engine pins, where
  no compatible patched release is available within 90 days.
- The crate is abandoned (no maintainer commits for >12 months
  while open security-relevant issues exist).
- The dalek-cryptography family's licensing changes
  incompatibly with the engine's MIT/Apache-2.0 stance.

None of these conditions held at audit close (2026-05-24).

### CI-side enforcement

The audit-remediation CI changes (in `.github/workflows/ci.yml`'s
gate job) enforce:

- **`Cargo.lock` is committed.** The lockfile pins exact versions
  including transitive deps. Reproducible-build cadence (ADR-052)
  verifies lockfile stability.
- **`deny.toml` license + advisory check.** `cargo-deny`'s
  `licenses` and `advisories` sections cover the three crypto
  crates. A known CVE on the pinned version triggers an
  advisory failure.
- **`cargo-semver-checks`** runs on every PR for the
  engine-api surface (ADR-012 / ADR-050).
- **`cargo-geiger`** (ADR-058) enumerates unsafe code in the
  crypto crates; their baseline counts are in
  `docs/observatory/cargo-geiger-baseline.md`. The baseline
  changes by PR review.

## Rationale

R-02 says owning every layer means owning every bug — and for
cryptography that is precisely the argument *against* owning it.
A subtle bug in an owned SHA-256 is a silent content-addressing
corruption; a subtle bug in an owned Ed25519 is a signature-
forgery vulnerability. These crates are constant-time,
test-vector-verified, fuzzed, and audited by a far larger
community than this project can field. Re-implementing them
would add risk, not remove it.

The determinism concern that motivates owning `engine-math` does
not apply here: cryptographic hashes are defined over exact
integer operations and are bit-reproducible by construction,
on every platform, with no floating point involved.

All three crates are permissively licensed (Apache-2.0 / MIT /
BSD-3-Clause) and pass the existing `deny.toml` policy.

## Consequences

- `engine-core` depends on `blake3`; `engine-asset` depends on
  `sha2` and `ed25519-dalek`. These are the foundation layer's
  only cryptographic dependencies.
- The engine still owns the *use* of these primitives — the RNG
  keying scheme (ADR-057), the content-address format (ADR-008),
  the pak signature envelope are all owned code. Only the
  primitive arithmetic is delegated.
- The `Cargo.lock` is committed; every PR pins exact versions.
- `cargo-deny`'s license + advisory check is the standing gate;
  a CVE against a pinned version is a CI failure.
- `cargo-geiger`'s baseline (ADR-058) captures the unsafe
  surface of these crates; new unsafe code in upstream is
  visible in the baseline diff.
- This ADR is the standing exception to R-02. Any future "own
  this layer" decision that touches cryptography is measured
  against it.

## Risks and tradeoffs

- **Upstream maintenance risk.** Mitigation: gate condition (12-
  month abandonment + open security issue) triggers replacement.
- **License churn.** Mitigation: `cargo-deny` allowlist; PR
  review on any license change in the lockfile.
- **CVE timing.** A 90-day grace window before forcing
  replacement is the policy; aggressive cases (active
  exploitation) trigger immediate response.
- **Audit re-verification cost.** Each crate's audit history
  must be reviewable; the references in this ADR are the
  starting points.

## Alternatives considered

- **Own SHA-256.** Audit-grade SHA-256 is a small program (~150
  LOC); the side-channel-resistance and SIMD-tuning that make
  the deployed implementation production-grade are not. The
  silent-bug risk is higher than the benefit. Rejected.
- **Own BLAKE3.** Same reasoning. The reference C BLAKE3 is
  the authoritative implementation; the Rust `blake3` crate is
  the maintained Rust binding. Rejected.
- **Own Ed25519.** Side-channel risk + signature-forgery
  consequences make this the worst candidate for owned
  implementation. Rejected.
- **Use a different audited crate** (e.g. `ring` for Ed25519
  + SHA-256). Considered; the dalek family + RustCrypto +
  BLAKE3 Team triple has stronger Rust ecosystem coverage
  and simpler licensing. Rejected for triple-family
  consolidation.

## Verification

- `cargo build --workspace --release` succeeds with the pinned
  versions.
- `cargo deny check` green in CI (license + advisory).
- `cargo-geiger --workspace` baseline in
  `docs/observatory/cargo-geiger-baseline.md` (ADR-058).
- ADR-052's reproducibility cadence verifies the lockfile is
  reproducible from source.
- A future "deliberately deprecate a version" test could be
  added (Phase 10+) to confirm the upgrade path; today the
  pin discipline is sufficient.
