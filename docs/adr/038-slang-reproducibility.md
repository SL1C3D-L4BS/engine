# ADR-038 · Slang shader output reproducibility

- Status: Accepted (PR 4 of Phase 4)
- Date: 2026-05-20
- Phase: 4 (SCRIPTING — spec Part XXI)

## Context

Sliced Engine's asset pipeline is content-addressed (ADR-008): the
same input bytes produce the same pak entry across runs and across
architectures. The cross-arch determinism contract (Phase 3,
ADR-013) already proves byte-equal output for engine-math goldens,
engine-core scheduler goldens, the sli compile golden (PR 1,
ADR-034), and the sli VM oracle (PR 2, ADR-035).

ADR-037 wraps `slangc` to compile shaders to SPIR-V / WGSL / DXIL /
MSL. The same determinism property must hold for the bytes
`slangc` writes: a Linux x86-64 runner and an aarch64 runner
compiling the same `.slang` against the same `slangc` version must
agree byte-for-byte. Without that, two cache-warm builds of the
same game produce different paks and the content-addressed store
sees two distinct entries for "the same" shader.

## Decision

### Version pin

`SLANGC_PIN = "v2026.9"` is the canonical version. `Compiler::locate`
refuses to run if the installed binary reports a different version;
`Compiler::permissive` opts out of the check but loses the golden
comparison.

Bumping `SLANGC_PIN` is a deliberate four-step act:

1. Update the constant in `tools/engine-shader/src/slangc.rs`.
2. Regenerate the golden:
   `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-shader --test reproducibility`.
3. Verify the digests still match on the new version's output (they
   should — the property is "two runs of the same version produce
   the same bytes", not "different versions produce the same
   bytes").
4. Commit (1)+(2) together; the ADR's date line should be touched
   in the same change.

### Golden format

`tools/engine-shader/tests/goldens/triangle-reproducibility.golden`:

```text
# engine-shader reproducibility golden (ADR-038)
# SLANGC_PIN = v2026.9
Stage   entry   Target  blake3_hex
...
```

Tab-delimited, sorted by `(stage, entry, target)`, one entry per
successfully-compiled (entry × target) pair. A target unavailable
on the runner (DXIL on Linux, missing `dxcompiler`) is *skipped*
from the comparison — the oracle never fails because a backend is
absent. The CI matrix records which platforms successfully
populated which lines.

### Cross-arch parity

Two architectures agreeing with one committed golden proves
cross-arch byte-equality transitively (the same pattern engine-math
and the sli compile golden use). The CI determinism job runs
`cargo test -p engine-shader --test reproducibility` on both
x86-64 and aarch64 runners; if `slangc` v2026.9 binaries exist for
both, the digest comparison enforces parity. If only one arch has
`slangc` available, that arch enforces and the other arch
contributes the smoke pass.

### What is *not* reproducible

- The `slangc` reflection JSON includes a `SLANG_GENERATOR`-style
  version string. We do **not** key the reproducibility digest on
  the reflection bytes; only the compiled output bytes are part of
  the contract. Reflection is metadata the renderer interprets; if
  `slangc` widens the JSON shape between patch releases, the
  bundle file changes but the artefact digest does not.
- WGSL and MSL outputs are text. They are still expected to be
  byte-equal across two runs of the same `slangc` version on the
  same source; the golden digests prove that.

## Consequences

### Positive

- Pak entries for shaders are content-addressable.
- The compile-once-distribute-many model holds for shaders.
- Cross-arch parity is mechanically checked.
- DXIL absence on Linux runners degrades gracefully — the rest of
  the matrix still enforces.

### Negative

- A toolchain bump is a coordinated change. The four-step recipe
  above is the cost of admission.
- `slangc` itself is large; the version pin slows the moment when
  a host installs the canonical version, but does not slow steady-
  state builds.

## See also

- ADR-008 — content-addressed asset pipeline.
- ADR-013 — cross-arch determinism contract.
- ADR-037 — Slang toolchain subprocess shape.

## Owner

Sliced Engine team. PR 4 in the Phase-4 sequence; closes Phase 4.
