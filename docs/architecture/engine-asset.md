# engine-asset

The content-addressed asset pipeline core (spec IV.1 Level 1, IV.8).

## Purpose

Identifies every compiled asset by the hash of its bytes, so the pipeline
deduplicates, caches, and delta-patches deterministically. Assets ship inside
signed `pak` archives that overlay for Live Ops.

## Modules

| Module    | Contents |
|-----------|----------|
| `hash`    | `ContentHash` — SHA-256 content addressing, with a hex round-trip for manifests. |
| `store`   | `ContentStore` — a deduplicating blob store; identical bytes hash to one key and one stored copy. |
| `pak`     | `Pak` archives (deterministic serialized form, integrity-checked on decode) and `PakSet` — newest-first overlay resolution with a per-name kill-switch. |
| `handle`  | `Handle<T>` typed handles and the `AssetServer` — load, dedup, ref-count, and hot-reload. |
| `sign`    | `PakSigner` / `verify` — Ed25519 signing of pak archives (ADR-025). |

## Design notes

- A `pak`'s serialized form is sorted and therefore byte-deterministic: the
  same inputs always produce the same archive, which is what makes signing and
  delta-patching meaningful.
- `PakSet` resolves names newest-pak-first; a broken asset is kill-switched by
  name without shipping a patch (Live Ops, spec IV.8).
- `AssetServer` hot-reload swaps the value inside a slot, so handles held since
  before the reload observe the new asset with no invalidation.
- Cryptography is delegated to audited crates, not owned (ADR-025).

## Out of scope

Format-specific importers (glTF, PNG, Slang) and the versioned `.scn` / `.sav`
formats — they need concrete component and scene types that do not exist yet.

## Oracle

`tests/pipeline.rs` — content-address reproducibility and deduplication, pak
sign → verify round-trip (and tamper detection), overlay newest-wins
resolution with the kill-switch, and the end-to-end `AssetServer` hot-reload
path.

## Dependencies

`engine-platform`, `sha2`, `ed25519-dalek` — Level 1.
