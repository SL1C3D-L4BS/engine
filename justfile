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

# Full pre-push gate.
ci: build test lint fmt-check deny
    @echo "[ENGINE] CI gate passed"
