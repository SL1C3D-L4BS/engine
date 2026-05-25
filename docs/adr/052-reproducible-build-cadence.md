# ADR-052 — Reproducible-build verification cadence

- Status: Accepted (CI workflow lands as part of audit remediation;
  first run scheduled within one week of merge)
- Date: 2026-05-24
- Phase: 5-prep
- Companion: ADR-008 (content-addressed asset pipeline — same
  reproducibility ethos), ADR-013 (Determinism Contract — same
  cross-arch byte-equality property), ADR-038 (Slang reproducibility
  golden — per-asset reproducibility precedent)

## Context

Spec §XX.8 (Build System Specification) demands:

> Reproducible builds: bit-identical artifacts verified weekly
> (SHA-256 same-commit-twice-cold-cache comparison).

This is one of the most concrete promises in the spec and one of
the least operationalised in the repo. The audit (§13.6) flagged it
as not-yet-wired. ADR-038 already proves per-asset reproducibility
for Slang shaders; this ADR extends the principle to the *whole-
binary* build.

The reproducible-build property is what lets a downstream consumer
(a Hub operator, a security auditor, a forensic investigator)
verify that a binary they hold was indeed produced by a specific
commit on a known toolchain — not silently tampered with at the
release stage.

## Decision

### 1. Cadence — weekly scheduled GH Actions workflow

A new workflow file `.github/workflows/reproducible-build.yml` runs
on a `schedule:` trigger every Sunday at 02:00 UTC, plus
manual-trigger via `workflow_dispatch`. (Sunday low-traffic for
both GitHub-hosted runners and the self-hosted GPU runner.)

The schedule is weekly because:

- Daily would over-spend CI minutes (each run is ~30 min wall-clock).
- Monthly is too slow to catch toolchain drift (a Rust release
  every 6 weeks could silently break reproducibility for 4 weeks
  before detection).
- Weekly matches the spec's stated cadence.

### 2. Method — two cold-cache builds of the same commit, compared

```yaml
jobs:
  reproducible_build:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - name: Install build tooling
        run: sudo apt-get update && sudo apt-get install -y mold clang

      # Build 1 — cold cache, default profile
      - name: Build 1 (cold)
        env:
          RUSTC_WRAPPER: ""
          CARGO_TARGET_DIR: /tmp/build1
        run: cargo build --release --workspace --locked

      - name: Hash artifacts (build 1)
        run: |
          find /tmp/build1/release -maxdepth 1 \
            -type f \( -name 'engine-*' -o -name 'libengine_*' \) \
            -exec sha256sum {} \; | sort > /tmp/sha-build1.txt
          cat /tmp/sha-build1.txt

      # Build 2 — cold cache (different target dir), default profile
      - name: Build 2 (cold)
        env:
          RUSTC_WRAPPER: ""
          CARGO_TARGET_DIR: /tmp/build2
        run: cargo build --release --workspace --locked

      - name: Hash artifacts (build 2)
        run: |
          find /tmp/build2/release -maxdepth 1 \
            -type f \( -name 'engine-*' -o -name 'libengine_*' \) \
            -exec sha256sum {} \; | sort > /tmp/sha-build2.txt
          cat /tmp/sha-build2.txt

      - name: Compare
        run: |
          if ! diff -q /tmp/sha-build1.txt /tmp/sha-build2.txt; then
            echo "::error::Reproducible-build property violated — see ADR-052"
            diff /tmp/sha-build1.txt /tmp/sha-build2.txt
            exit 1
          fi
          echo "All hashes match — reproducible build confirmed."

      - name: Log to observatory
        if: always()
        run: |
          mkdir -p /tmp/log
          {
            echo "## $(date -u +%Y-%m-%d) — commit ${GITHUB_SHA:0:12}"
            echo
            echo '```'
            cat /tmp/sha-build1.txt
            echo '```'
            echo
            if cmp -s /tmp/sha-build1.txt /tmp/sha-build2.txt; then
              echo "Result: REPRODUCIBLE"
            else
              echo "Result: DIVERGENT"
              echo
              echo '```diff'
              diff /tmp/sha-build1.txt /tmp/sha-build2.txt || true
              echo '```'
            fi
          } > /tmp/log/run.md
      - uses: actions/upload-artifact@v4
        with:
          name: reproducible-build-${{ github.run_id }}
          path: /tmp/log/run.md
```

The run produces an artifact (`reproducible-build-<run_id>`)
containing the human-readable log. The log is *not* committed
automatically (avoid noisy commit history); a follow-up manual
commit appends accumulated runs to
`docs/observatory/reproducible-build-log.md` periodically (monthly
rollup).

### 3. Scope — workspace release artifacts only

The hash comparison covers:

- All `target/release/engine-*` binaries (engine-debug, engine-repl,
  engine-shader, sampling-profiler, cache-observatory, future
  binaries).
- All `target/release/libengine_*.rlib` library artifacts (the
  Rust-side "binary" form of every workspace crate).

Not covered (intentionally):

- Test binaries (per-test, not stable artifacts).
- Debug-profile builds (debug info contains paths and timestamps;
  spec only promises release).
- Dependency artifacts (libstd, third-party crates) — reproducibility
  of those is the Rust toolchain's responsibility, not the engine's.

### 4. Toolchain pinning

`rust-toolchain.toml` already pins the channel and components. The
workflow installs nothing toolchain-related on top — `rustup`
honours the pin. This guarantees both builds use the same compiler.

### 5. Hardware envelope

`ubuntu-24.04` (x86-64, GitHub-hosted). The reproducible-build
property is a *same-machine* property; cross-architecture
reproducibility is the *Determinism Contract* (ADR-013) and is
tested by the determinism job's cross-arch goldens.

The reproducible-build job runs only on x86-64; same-commit
byte-equality across runs on the *same* runner family is the
verified property.

### 6. Failure handling

On a divergence:

- The CI job fails (red status on the workflow page).
- The structured log artefact captures the diff for inspection.
- A follow-up issue is opened (manual; the workflow does not
  auto-create issues to avoid PR-bot noise).
- The runbook in `docs/runbooks/reproducible-build-divergence.md`
  (lands with this ADR's CI workflow) walks through the diagnosis
  steps: dependency drift (Cargo.lock change?), toolchain drift
  (rustup channel update?), environment drift (env var leak?).

### 7. First run within one week of merge

The first run lands on the next Sunday after the workflow PR
merges. Subsequent runs are weekly. The first three runs are
manually verified before the workflow is considered "trusted" — a
divergence in the first runs typically reflects a workflow bug, not
a reproducibility bug.

## Consequences

- One new workflow file. Cost: ~30 min × 4 runs/month ≈ 2 hours/
  month of GH Actions minutes. Negligible.
- One new directory: `docs/runbooks/`, with one file. The runbook
  becomes the locus for "things to do when this specific CI job
  goes red" — separate from per-PR triage runbooks.
- One new accumulating doc: `docs/observatory/reproducible-build-
  log.md`. First entry committed manually as part of the
  remediation packet.
- Engineering discipline reminder: a Cargo.toml change that
  introduces a non-deterministic build dependency (e.g. one that
  embeds timestamps) will surface as a reproducible-build failure
  the following Sunday. Worth catching.

## Risks and tradeoffs

- **GitHub-hosted runners are shared.** Same-runner-family
  reproducibility holds across two builds on the same job because
  GH Actions provisions a fresh VM per job (and we use two
  separate target dirs to guarantee cold cache for both). The risk
  is that GitHub silently changes the runner image; this would
  surface as a reproducibility failure tied to a specific date,
  and the runbook covers it.
- **Cargo.lock churn from `cargo update`** breaks reproducibility
  if not pinned. Mitigation: the `--locked` flag in the workflow
  refuses to build without an up-to-date lockfile; a developer
  who skips `cargo update` review will see the workflow fail.
- **Macros that embed timestamps** (`std::env!("CARGO_PKG_*")`,
  `chrono::Local::now()`-at-build-time) are the classical
  reproducibility killer. None known in current crate code; if
  one lands, the workflow catches it the next Sunday.
- **Reproducibility across distros / glibc versions** is the
  spec's *cross-arch* promise (Determinism Contract), not this
  ADR's promise. This ADR is the per-runner same-commit-twice
  baseline; the determinism job is the cross-arch contract.

## Alternatives considered

- **Verify on every push to main.** Too expensive in CI minutes;
  the spec asks for weekly. Rejected.
- **Use `reprotest` (Debian Reproducible Builds toolkit).** A
  richer tool, varies many environmental dimensions
  (umask, locale, time, …). Overshoots the spec's stated method.
  Rejected for now; a Phase-10+ candidate when the engine has a
  binary-distribution story.
- **Auto-commit the log to the repo** instead of artefact-upload.
  Noisy git history; rejected. Monthly manual rollup is the
  compromise.

## Verification

- Lands as part of the audit-remediation CI-extensions packet
  (task #17). PR ships:
  - One new workflow file.
  - One new runbook file.
  - First entry in `docs/observatory/reproducible-build-log.md`.
- First scheduled run on the following Sunday — observed manually;
  result added to the log.
- Steady state: weekly green run, monthly manual rollup of
  artefact contents to the observatory log.
