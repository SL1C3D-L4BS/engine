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

# Full pre-push gate.
ci: build test lint fmt-check deny
    @echo "[ENGINE] CI gate passed"
