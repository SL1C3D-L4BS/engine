# Runbook — reproducible-build divergence

When `.github/workflows/reproducible-build.yml` reports a DIVERGENT
result, two cold-cache builds of the *same commit* produced *different*
binary artefacts. Spec §XX.8 promises bit-identical builds; this is a
contract violation. ADR-052 is the controlling decision.

## 1. Triage

Open the workflow run page and download the
`reproducible-build-<run-id>` artefact. It contains:

- The sorted SHA-256 hash list for build 1 and build 2.
- The diff between the two when they disagree.

Identify which artefacts diverged. Typical patterns:

- **One specific binary** — a macro or build script in *that* crate
  embedded non-deterministic state. Search the crate for `env!`,
  `option_env!`, `std::time`, `chrono`, `SystemTime`, `proc_macro` use.
- **Every binary** — a workspace-wide change embedded non-determinism
  (e.g. a workspace-level build script). Search top-level `build.rs`
  and `Cargo.toml` `[package.metadata.*]` sections.
- **Same crate's two outputs** (e.g. an executable + its `.rlib`) —
  almost always linker- or codegen-level. Check `RUSTFLAGS`,
  `[profile.release]` settings.

## 2. Diagnose

Reproduce locally:

```sh
CARGO_TARGET_DIR=/tmp/build-a cargo build --release --workspace --locked
CARGO_TARGET_DIR=/tmp/build-b cargo build --release --workspace --locked
find /tmp/build-{a,b}/release -maxdepth 1 -type f \( -name 'engine-*' -o -name 'libengine_*' \) \
    -exec sha256sum {} \;
```

If local builds agree but CI disagrees, the problem is in the CI
environment (runner image change, dependency hash drift). If local
builds *also* disagree, the problem is in-tree code or `Cargo.lock`.

### Drill down on a single binary

```sh
diff <(objdump -s /tmp/build-a/release/engine-debug) \
     <(objdump -s /tmp/build-b/release/engine-debug)
```

The offset of the first diverging byte is a strong hint. Section
`.rodata` divergence → embedded data (likely a macro). Section `.text`
divergence → codegen ordering (likely a non-deterministic `HashMap`
iteration in a build script or proc-macro).

## 3. Fix

Common fixes by pattern:

| Pattern | Fix |
|---|---|
| `env!("CARGO_PKG_*")` embedding | Allowed — these are pinned at build time. |
| `std::time::SystemTime::now()` in a `build.rs` | Replace with a pinned epoch (e.g. `1735689600 // 2025-01-01 UTC`). |
| `chrono::Local::now()` in a proc-macro | Same as above. |
| Iterating a `std::collections::HashMap` for codegen output | Use a deterministic structure (BTreeMap, sorted Vec). |
| `Cargo.lock` drift between runs | Pin `--locked` (already on the workflow); if still drifting, audit dependency overrides in `Cargo.toml`. |
| Linker order from `[lib]` and `[[bin]]` interleaving | Avoid; if needed, accept and document the exception via ADR amendment. |

## 4. Verify the fix

Re-run the workflow via "Run workflow" on the workflow page. The
result must read REPRODUCIBLE for the fix to land.

For sensitive fixes (e.g. embedded-timestamp removal), pin the change
behind a PR explicitly tagged with a reference to this runbook so a
reviewer follows the same logic.

## 5. Update the observatory log

Once the run is green, append the resolved entry to
`docs/observatory/reproducible-build-log.md`:

```markdown
### YYYY-MM-DD — divergence resolved
Commit: <12-char SHA>
Symptom: <one-line>
Root cause: <one-line>
Fix: <one-line + PR link>
```

Monthly rollups consolidate the per-run artefacts into this log.

## 6. Escalation

If three consecutive weekly runs go red with different root causes,
the toolchain itself may have drifted. Pin `rust-toolchain.toml` to a
specific known-good `channel = "X.Y.Z"` until the source of churn is
identified. Re-evaluate after the next stable Rust release.

— *This runbook is referenced by ADR-052.*
