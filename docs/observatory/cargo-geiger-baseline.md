# cargo-geiger baseline

Authoritative enumeration of `unsafe` code in the engine
workspace (ADR-058). The baseline is the committed source of
truth; each PR that adds or removes unsafe items updates this
file in the same change.

## Format

```text
<crate> | unsafe_fns: <N> | unsafe_methods: <N> | unsafe_impls: <N>
        | unsafe_blocks: <N> | naked_asm: <N>
        Notes: <short rationale + ADR link>
```

`unsafe_blocks` counts statement-level `unsafe { ... }` blocks
(approximate). `naked_asm` counts `core::arch::naked_asm!`
macro invocations (ADR-032 fiber-only).

## Baseline as of 2026-05-24 (Phase 4 audit close)

### engine-core
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 1 | naked_asm: 0
- Notes: the single scheduler reborrow in `ecs::schedule::dispatch_phase`
  (ADR-033). Justified by R/W declaration discipline; backstopped
  by the replay-parity oracle.

### engine-platform
- unsafe_fns: ~6 | unsafe_methods: ~4 | unsafe_impls: 0
- unsafe_blocks: ~14 | naked_asm: 2
- Notes: fiber switch in `fiber/{x86_64,aarch64}.rs` (ADR-032);
  mmap+munmap discipline in `mmap.rs` (ADR-029); sampler perf-
  event ring buffer in `sampler.rs` (ADR-030). Each unsafe site
  has a dedicated oracle.

### engine-script
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: pure-safe Rust. VM dispatch uses indexed array access
  with verifier-guaranteed bounds (ADR-035); no unsafe required.

### engine-asset
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: routes through `engine_platform::mmap::MmapRo` for all
  mmap (ADR-029).

### engine-math
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: pure-safe. SIMD via `std::simd` portable backend (ADR-027).

### engine-telemetry
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: owned compact binary encoder; no unsafe.

### engine-ecs-macro
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: proc-macro; no unsafe in generated output (ADR-024).

### engine-reflect
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: pure-safe reflection.

### engine-i18n
- unsafe_fns: 0 | unsafe_methods: 0 | unsafe_impls: 0
- unsafe_blocks: 0 | naked_asm: 0
- Notes: owned Fluent-subset parser + CLDR plural rules (ADR-051
  deviation), no unsafe.

### engine-render (stub at audit close)
- 0 across the board.

### engine-api (stub at audit close)
- 0 across the board.

### engine-* (all other upper-layer stubs)
- 0 across the board (audio, ai, net, physics, ui, editor,
  hub-core, plugin-api).

## Upstream dependency unsafe (visibility, not gate)

Workspace deps with non-trivial unsafe surface (reported by
geiger), captured here for visibility:

- `blake3` — SIMD-accelerated BLAKE3 implementation. Unsafe
  contained to the SIMD backends; portable backend is unsafe-
  free. ADR-025 audit reference.
- `sha2` — RustCrypto SIMD-accelerated SHA-256. Same pattern.
- `ed25519-dalek` / `curve25519-dalek` — group operations
  use unsafe for performance on hot paths. ADR-025 audit
  reference (Trail of Bits audited).
- `libc` — Unix syscall surface. The engine uses `libc::mmap`
  through engine-platform's owned wrapper (ADR-029) — direct
  usage outside that crate is rejected by the boundary guard.
- `wgpu` (Phase 5 PR 2+) — substantial unsafe surface in the
  vendor binding crates. Walled behind engine-gpu per ADR-049.

## Update discipline

Every PR that adds or removes unsafe in a workspace crate
updates this file. The diff is the review surface; the
explanation lives in the Notes section of the affected crate's
row.

PR-time check: `cargo run -p engine-geiger-check -- --baseline
docs/observatory/cargo-geiger-baseline.md --report geiger.json`
must exit 0.
