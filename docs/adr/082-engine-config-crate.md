# ADR-082 — `engine-config` Level-1 crate

- Status: Accepted (Phase 6 design contract; implementation lands in
  Phase 6 PR 1b)
- Date: 2026-05-28
- Phase: 6 — NEURAL RENDERING & GAUSSIAN SPLATTING
- Companion: ADR-051 (acknowledged deviations — entry 1 TOML for
  breakpoints stays in place), ADR-068 PR 7.5 (the finding #15
  audit observation this ADR closes), ADR-084 (Phase 6 PR slicing)

## Context

The Phase 5.5 PR 7.5 audit surfaced finding #15: three crates each
ship an owned line-oriented TOML reader with nearly identical
mechanics:

| File | LOC | Purpose |
|---|---|---|
| `crates/engine-render/src/upscaler_config.rs` | 394 | parses `[upscaler]` from `engine.toml` |
| `bin/engine-bench-frame-pacing/src/budgets.rs` | ~150 | parses `tools/frame-pacing/budgets.toml` |
| `crates/engine-script/src/breakpoints_toml.rs` | ~144 | parses `.engine/debug/breakpoints.toml` |

Each implements section-aware key-value extraction with quote-aware
comment stripping. They share the same edge cases (quoted `#`
inside strings, escaped backslashes, bracketed section trimming);
each has its own slightly-different test coverage. A consolidating
shared crate is straightforward.

The shared crate is *not* a third-party TOML library. ADR-051
entry 1 acknowledged the owned TOML parser deviation as a
deliberate choice; the audit's finding #15 is about *DRY*, not
about replacing the owned implementation with a vendored one.

## Decision

### 1. New Level-1 crate `engine-config`

```
crates/engine-config/
├── Cargo.toml             # level 1: depends on stdlib only
└── src/
    ├── lib.rs             # public surface
    ├── lex.rs             # owned line-oriented tokenizer
    ├── parse.rs           # section-aware parser
    └── tests/
        └── parse_smoke.rs # round-trip + edge-case coverage
```

Workspace level 1: no engine-* deps except `engine-math`
(transitive — `engine-config` itself depends only on `core` +
`alloc`). The crate has no GPU, no script, no render dependencies.

### 2. Public API

```rust
// crates/engine-config/src/lib.rs

pub struct Config {
    sections: Vec<Section>,
}

pub struct Section {
    pub name: String, // empty for the implicit root section
    pub entries: Vec<(String, Value)>,
}

pub enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}

#[derive(Debug)]
pub enum ParseError {
    UnterminatedString { line: usize, col: usize },
    MalformedSectionHeader { line: usize },
    DuplicateKeyInSection { line: usize, key: String },
    EmptyKey { line: usize },
}

pub fn parse(input: &str) -> Result<Config, ParseError>;

impl Config {
    pub fn section(&self, name: &str) -> Option<&Section>;
    pub fn get(&self, section: &str, key: &str) -> Option<&Value>;
}

// Quote-aware helpers (re-used by call sites that need them).
pub fn strip_comment(line: &str) -> &str;
pub fn unquote(input: &str) -> Option<String>;
```

The Value enum is intentionally narrow (no arrays, no tables, no
inline tables, no dotted keys) — TOML in the engine's use is a
flat key-value store organised by `[section]`. The three call
sites' existing schemas all fit this narrowing without churn; the
*full* TOML spec is not the contract.

### 3. Three call sites become thin adapters

```rust
// crates/engine-render/src/upscaler_config.rs (was 394 LOC)
pub fn parse_upscaler_config(input: &str) -> Result<UpscalerConfig, ConfigError> {
    let cfg = engine_config::parse(input)?;
    let Some(section) = cfg.section("upscaler") else { return Ok(Default::default()) };
    // ... map keys ("provider", "quality") to UpscalerConfig fields ...
}
```

Each call site keeps its module-local strongly-typed config struct
and an error type that wraps `engine_config::ParseError`. The
parser logic itself is gone; the adapter is ~50 LOC per call site.

Net change:
- `upscaler_config.rs` from 394 LOC → ~150 LOC (adapter + type
  definitions + tests)
- `budgets.rs` from ~150 LOC → ~60 LOC
- `breakpoints_toml.rs` from ~144 LOC → ~50 LOC

Plus the new `engine-config` crate at ~400 LOC (lex + parse +
tests). The net workspace LOC delta is roughly -300 LOC.

### 4. Public API stability

Per ADR-012 (50-year API stability contract), `engine-config`'s
public surface is small and disciplined:

- `Config::section()` and `Config::get()` use string keys.
- `Value`'s four variants are the universal lower bound of
  what every call site needs.
- New variants (arrays, tables) require a new `engine-config`
  major version + an ADR amendment. Not anticipated.

### 5. CI boundary guard

`.github/workflows/ci.yml` grows a grep guard in the
workspace-boundary section:

```yaml
- name: Reject new ad-hoc TOML parsers
  run: |
    set -euo pipefail
    if grep -rn --include='*.rs' --exclude-dir='engine-config' \
       -E 'fn (parse_section|strip_comment|unquote).*->.*Section' \
       crates/ bin/ tools/ testbed/; then
        echo "::error::ad-hoc TOML parser found outside engine-config; consolidate per ADR-082"
        exit 1
    fi
```

The guard is strict but coarse (looks for the function names this
ADR's parser uses); it rejects new instances without locking
existing call sites that have been ported.

### 6. ADR-051 entry 1 unchanged

The breakpoint format remains TOML (per ADR-051 entry 1's
acknowledged deviation). `engine-script` continues to use TOML
for `.engine/debug/breakpoints.toml`; this ADR merely consolidates
*how* TOML is parsed, not *which format* is used. ADR-051's
"gate condition" (if engine-script adopts RON for another purpose,
revisit) stays in place.

## Consequences

### Positive

- One TOML parser to maintain, not three.
- Public API is small + frozen; future call sites pick up the
  shared parser by adding one workspace dep.
- The line-oriented tokenizer pattern is preserved (per ADR-051);
  no third-party TOML library enters the workspace.
- Boundary guard catches drift.

### Negative

- A new Level-1 crate widens the workspace by one member.
  Acceptable: Level-1 crates are the foundation layer; adding one
  is rare but not unprecedented (`engine-i18n`, `engine-math`,
  `engine-platform` are siblings).

### Neutral

- Each call site keeps its strongly-typed config struct. The
  adapter pattern preserves the existing public API of each
  consumer; no upstream churn.

## Implementation

PR 1b of Phase 6 (per ADR-084):

1. `crates/engine-config/` — new crate with the public surface
   above.
2. `Cargo.toml` workspace member entry + workspace-deps entry.
3. `crates/engine-render/src/upscaler_config.rs` — adapter.
4. `bin/engine-bench-frame-pacing/src/budgets.rs` — adapter.
5. `crates/engine-script/src/breakpoints_toml.rs` — adapter.
6. `.github/workflows/ci.yml` — boundary grep guard.
7. `crates/engine-config/tests/parse_smoke.rs` — round-trip +
   edge-case coverage.

## References

### Prior engine ADRs

- [ADR-012](012-50-year-api-stability-contract.md) — the public
  API discipline this crate inherits.
- [ADR-051](051-acknowledged-deviations.md) — entry 1's
  acknowledgement of the owned TOML reader stays in place.
- [ADR-068](068-phase-6-pr-slicing.md) — finding #15 (PR 7.5
  audit) is the trigger for this ADR.
- [ADR-084](084-phase-6-pr-slicing.md) — Phase 6 PR slicing.
