# [ENGINE] platform — task runner
# CI gate mirrors the spec XVIII.14 pre-push checks.

default:
    @just --list

build:
    cargo build --workspace

test:
    cargo nextest run --workspace --no-tests=pass

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt-check:
    cargo fmt --all --check

deny:
    cargo deny check

semver:
    cargo semver-checks check-release --workspace

# Run the cross-architecture determinism oracles on the `sim` math path
# (FMA disabled), comparing against the committed golden files (ADR-023).
determinism:
    RUSTFLAGS="-C target-feature=-fma" cargo test -p engine-math --test determinism
    RUSTFLAGS="-C target-feature=-fma" cargo test -p engine-core --test determinism

# Regenerate the determinism golden files. Run once, on x86-64, after an
# intentional change to a determinism oracle — then commit the new goldens.
gen-golden:
    ENGINE_GOLDEN_WRITE=1 cargo test -p engine-math --test determinism
    ENGINE_GOLDEN_WRITE=1 cargo test -p engine-core --test determinism

# Run the workspace benchmarks (Phase 1 arena benches). Not part of `ci` —
# bench numbers are too runner-noisy to gate on, but we keep them runnable.
bench:
    cargo bench --workspace

# Generate a cache-observatory baseline report (wall-clock only).
cache-baseline:
    cargo run --release -p cache-observatory

# Generate a cache-observatory baseline with kernel perf counters. Requires
# perf_event_paranoid <= 2 or CAP_PERFMON; falls back to wall-clock otherwise.
cache-baseline-with-counters:
    cargo run --release -p cache-observatory -- --with-perf-counters

# Refresh the Robin Hood hash-map criterion baseline (ADR-028). Bench numbers
# land in `target/criterion/`; commit summary numbers to
# `docs/observatory/hashmap-baseline.md`.
hashmap-baseline:
    cargo bench -p engine-core --bench collections

# Refresh the mmap'd-pak loader baseline (ADR-029). Builds a 256 MiB synthetic
# pak (~10 000 blobs) on tmpfs and compares Pak::from_bytes(fs::read(path))
# against Pak::open_mmap(path). Commit the summary to
# `docs/observatory/mmap-asset-baseline.md`.
mmap-baseline:
    cargo run --release -p engine-asset --example mmap_baseline

# Run the sampling profiler oracle (ADR-030). Requires Linux, frame-pointer
# codegen, and an optimized build — the oracle's "spinner ≥ 80% self-time"
# threshold only holds with optimizations on.
profiler-oracle:
    RUSTFLAGS="-C force-frame-pointers=yes" cargo test --release -p engine-telemetry --test profiler_oracle

# Refresh the sampling-profiler baseline (ADR-030). Reports overhead and
# sample-drop rate at 99/199/499/997 Hz. Commit summary numbers to
# `docs/observatory/profiler-baseline.md`.
profiler-baseline:
    RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release -p sampling-profiler
    @for hz in 99 199 499 997; do \
      echo "## $$hz Hz" ; \
      ./target/release/sampling-profiler --rate-hz $$hz --duration-s 1 --workload arena_alloc > /dev/null ; \
    done

# Refresh the archetype-traversal observatory baseline (ADR-031). The
# `archetype-traversal` cache-observatory mode lands with the PR 3
# million-entities harness; until then this recipe is a placeholder that
# mirrors the other observatory recipes.
archetype-baseline:
    cargo run --release -p cache-observatory -- --workload archetype-traversal

# Run the engine-core archetype oracle (ADR-031): adjacency moves, both
# storage backends, swap-remove correctness.
archetype-oracle:
    cargo test -p engine-core --test archetype

# Run the engine-platform jobs oracle (ADR-032): single-threaded reference
# vs parallel pool digests across worker counts {1, 2, 4, N}.
jobs-oracle:
    cargo test -p engine-platform --test jobs_oracle

# Refresh the jobs throughput baseline (ADR-032). Records dispatch overhead
# and throughput across linear / fan-out / mixed-grain graph shapes.
# Commit summary numbers to `docs/observatory/jobs-baseline.md`.
jobs-bench:
    cargo bench -p engine-platform --bench jobs

# Full pre-push gate.
ci: build test lint fmt-check deny
    @echo "[ENGINE] CI gate passed"
