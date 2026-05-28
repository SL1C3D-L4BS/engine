//! ADR-075 §Verification — pass-`record()` discipline grep test.
//!
//! Asserts that every concrete `Pass::record` body in
//! `crates/engine-render/src/passes.rs` follows the six-step template
//! documented in ADR-075 §1. The test parses `passes.rs` as a string
//! and pattern-matches each record() body shape against the contract:
//!
//! 1. CPU-oracle short-circuit on `ctx.gpu.as_mut()` returning `None`.
//! 2. Pipeline-installed short-circuit on `self.pipeline.as_ref()`
//!    (or the equivalent `self.pipeline_extract.as_ref()` for the
//!    bloom triple-pipeline pass) returning `None`.
//! 3. (Phase 5.5 A.2b-ii foundation: optional, A.2c full bodies must
//!    add) resolver short-circuit on `ctx.resources` returning `None`.
//! 4. begin/set/dispatch/draw — the actual GPU work, encoded across
//!    Steps 4-6 of the template (open pass scope, bind resources,
//!    issue work; end-of-scope drops the pass and the encoder is
//!    submitted by the graph executor).
//!
//! Discipline mode for the foundation commit: each `record()` body
//! either (a) contains the `ctx.gpu.as_mut()` short-circuit followed by
//! `set_pipeline` or `begin_render_pass`, demonstrating the active
//! template, OR (b) is an `_ctx: &mut PassContext` deliberate no-op
//! (the unused-arg convention names pre-A.2c not-yet-wired passes). A
//! `record()` body that calls `set_pipeline` without first installing
//! the Step-1 + Step-2 short-circuits fails the test loudly — that
//! shape would panic on missing pipeline on the per-frame hot path,
//! which PR 7.5 explicitly removed (ADR-075 §1 Step 2).

use std::fs;
use std::path::PathBuf;

/// Path to `passes.rs`. Resolved from `CARGO_MANIFEST_DIR` so the test
/// is independent of the cwd `cargo test` was invoked from.
fn passes_rs_path() -> PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    PathBuf::from(manifest).join("src").join("passes.rs")
}

/// Find every `fn record(...) {` body in the source and return
/// `(struct_name, body_text)` pairs.
///
/// `struct_name` is the immediately-preceding `impl Pass for X` line's
/// `X`; `body_text` is the brace-balanced body of the `fn record`.
/// Robust enough for the Phase-5.5 passes.rs shape (no nested fns inside
/// record(), no string literals with braces inside record()).
fn extract_record_bodies(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current_struct: Option<String> = None;
    let mut pos = 0;
    while pos < src.len() {
        // Find the next `impl Pass for ` first so we know which struct
        // we're inside.
        if let Some(impl_idx) = src[pos..].find("impl Pass for ") {
            let abs = pos + impl_idx + "impl Pass for ".len();
            let rest = &src[abs..];
            let end = rest
                .find(char::is_whitespace)
                .unwrap_or(rest.len())
                .min(rest.find('{').unwrap_or(rest.len()));
            current_struct = Some(rest[..end].trim().to_string());
            pos = abs + end;
        }
        // Find the next `fn record(` after the current impl.
        let fn_record = match src[pos..].find("fn record(") {
            Some(i) => pos + i,
            None => break,
        };
        // Bail if we ran past another `impl Pass for` between here and
        // the previous one — restart the loop to re-anchor.
        if src[pos..fn_record].contains("impl Pass for ") {
            continue;
        }
        // Find the opening brace of the function body.
        let body_open = match src[fn_record..].find('{') {
            Some(i) => fn_record + i,
            None => break,
        };
        // Brace-balance to find the closing brace.
        let mut depth = 0;
        let mut end = body_open;
        for (i, c) in src[body_open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = body_open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = src[body_open + 1..end].to_string();
        let name = current_struct
            .clone()
            .unwrap_or_else(|| "<unknown>".to_string());
        out.push((name, body));
        pos = end + 1;
    }
    out
}

/// Discipline shape: at least one of
/// - active GPU template (Step 1 + Step 2 + begin/set/dispatch present), or
/// - deliberate no-op (`_ctx` unused-arg convention).
#[derive(Debug, PartialEq, Eq)]
enum Shape {
    /// Body contains Step 1 + Step 2 short-circuits and a begin_*pass /
    /// set_pipeline call. This is the active template.
    Active,
    /// Body is a deliberate no-op marked by `_ctx: &mut PassContext`
    /// (unused-arg convention) — pre-A.2c not-yet-wired pass.
    DeliberateNoOp,
    /// Body shape doesn't match either acceptable form — discipline
    /// failure.
    Invalid,
}

/// Inspect `(struct_name, fn_record_signature_and_body)` and decide
/// which shape it follows.
///
/// `signature_and_body` includes the `fn record(self, ...) {` text +
/// the body; we use the signature to detect the `_ctx` convention.
fn classify(body: &str, fn_decl_window: &str) -> Shape {
    // Deliberate no-op: the function-signature window contains
    // `_ctx: &mut PassContext` (any leading underscore on the arg).
    if fn_decl_window.contains("_ctx") {
        return Shape::DeliberateNoOp;
    }
    // Active template: the body must contain both short-circuit
    // patterns AND at least one Step-5 GPU-work indicator. Direct
    // begin_*pass calls cover the inline case; helper calls
    // (`dispatch_bloom_stage` for the bloom mip chain) cover the
    // delegated case — the record() body still issues real GPU work,
    // just through a helper. The helper's name encodes its contract;
    // adding new helpers requires updating this list explicitly so
    // the discipline can't drift silently.
    let has_gpu_sc = body.contains("ctx.gpu.as_mut()");
    // UpscalePass (Phase 6 PR 1a, ADR-083) carries two pipelines —
    // bilinear + EASU — selected at record() time from the upscaler
    // registry; accept either-or-both as the short-circuit witness.
    let has_pipeline_sc = body.contains("self.pipeline.as_ref()")
        || body.contains("self.pipeline_extract.as_ref()")
        || body.contains("self.bilinear_pipeline.as_ref()")
        || body.contains("self.easu_pipeline.as_ref()");
    let has_gpu_work = body.contains("begin_compute_pass(")
        || body.contains("begin_render_pass(")
        || body.contains("begin_render_pass_desc(")
        || body.contains("dispatch_bloom_stage(");
    if has_gpu_sc && has_pipeline_sc && has_gpu_work {
        Shape::Active
    } else {
        Shape::Invalid
    }
}

/// Every `fn record()` body in passes.rs follows the ADR-075 §1
/// six-step template, either actively (with the Step-1 + Step-2
/// short-circuits and at least one begin_*pass call) or as a
/// deliberate `_ctx` no-op (pre-A.2c not-yet-wired pass).
#[test]
fn every_pass_record_follows_adr_075_template() {
    let path = passes_rs_path();
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("could not read {}: {e}", path.display());
    });
    let bodies = extract_record_bodies(&src);
    assert!(
        bodies.len() >= 10,
        "expected ≥ 10 Pass::record() bodies in passes.rs, found {}; \
         the extractor is broken or passes.rs structure changed",
        bodies.len()
    );
    let mut failures: Vec<String> = Vec::new();
    // For each record() body, also slice out a window around the
    // signature so we can detect `_ctx` deliberate no-op convention.
    for (name, body) in &bodies {
        // Find the body inside the file by string match; classify
        // using a window around the body start that includes the
        // function signature.
        let body_idx = src
            .find(body.as_str())
            .expect("body must be locatable in the source");
        let sig_start = src[..body_idx]
            .rfind("fn record(")
            .expect("signature must precede body");
        let sig_window = &src[sig_start..body_idx];
        let shape = classify(body, sig_window);
        if shape == Shape::Invalid {
            failures.push(format!(
                "pass {name:?}: record() body is neither the active \
                 ADR-075 §1 template (Step 1 ctx.gpu short-circuit + \
                 Step 2 pipeline short-circuit + begin_*pass) nor a \
                 deliberate `_ctx` no-op. Bring it into compliance \
                 or mark the arg `_ctx` to opt into the pre-A.2c \
                 not-yet-wired convention. Body excerpt:\n{}",
                body.lines().take(6).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "ADR-075 §1 discipline violations:\n{}",
        failures.join("\n\n")
    );
}

/// `extract_record_bodies` finds the expected 11 pass record() bodies
/// (10 Track-A passes + UpscalePass). Regression-locks the extractor
/// against passes.rs structural changes.
#[test]
fn extractor_locates_expected_record_count() {
    let path = passes_rs_path();
    let src = fs::read_to_string(&path).expect("read passes.rs");
    let bodies = extract_record_bodies(&src);
    let names: Vec<&str> = bodies.iter().map(|(n, _)| n.as_str()).collect();
    // 11 known passes: CullPass, CsmShadowPass, ClusterLightPass,
    // GBufferPass, LightingAccumulationPass, SsaoPass, IblPass,
    // TaaPass, BloomPass, UpscalePass, TonemapPass.
    for required in [
        "CullPass",
        "CsmShadowPass",
        "ClusterLightPass",
        "GBufferPass",
        "LightingAccumulationPass",
        "SsaoPass",
        "IblPass",
        "TaaPass",
        "BloomPass",
        "UpscalePass",
        "TonemapPass",
    ] {
        assert!(
            names.contains(&required),
            "extractor missed `{required}`; found: {names:?}"
        );
    }
}
