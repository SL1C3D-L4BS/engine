//! Cross-arch reproducibility oracle (PR 4, ADR-038).
//!
//! Compiles the fixture against every available target, computes a
//! BLAKE3 digest per (target, entry, stage), and compares against the
//! committed goldens under `tests/goldens/`. A target whose backend
//! is unavailable (e.g. DXIL on Linux) is *skipped*, not failed; the
//! golden file records the digests we expect under
//! `SLANGC_PIN = v2026.9`.
//!
//! Generating goldens: `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-shader
//! --test reproducibility`.

use engine_shader::artifact::Artifact;
use engine_shader::slangc::{Compiler, SLANGC_PIN, SlangcError};
use engine_shader::target::{Stage, Target};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/triangle.slang")
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/triangle-reproducibility.golden")
}

/// Entries to attempt — one per stage we exercise.
fn entries() -> Vec<(&'static str, Stage)> {
    vec![
        ("vs_main", Stage::Vertex),
        ("fs_main", Stage::Fragment),
        ("cs_main", Stage::Compute),
    ]
}

#[test]
fn reproducibility_matches_golden() {
    let compiler = match Compiler::permissive() {
        Ok(c) => c,
        Err(SlangcError::NotFound) => {
            eprintln!("slangc not installed — skipping reproducibility oracle");
            return;
        }
        Err(e) => panic!("locating slangc failed: {e}"),
    };

    // Goldens are pinned to the canonical slangc version; any other
    // version produces non-matching digests by design. The oracle
    // still runs the compile path to confirm the toolchain works,
    // but only enforces digests when the pin matches.
    let enforce_digests = compiler.version() == SLANGC_PIN;
    if !enforce_digests {
        eprintln!(
            "slangc version {} does not match SLANGC_PIN={SLANGC_PIN} — \
             skipping golden digest comparison (compiling for smoke only).",
            compiler.version()
        );
    }

    let mut digests: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for (entry, stage) in entries() {
        for &target in Target::all() {
            match compiler.compile(&fixture(), entry, stage, target) {
                Ok(bytes) => {
                    let a = Artifact::new(target, bytes, Vec::new());
                    digests.insert(
                        (
                            format!("{:?}", stage),
                            entry.to_string(),
                            format!("{:?}", target),
                        ),
                        a.digest_hex(),
                    );
                }
                Err(SlangcError::Compile { stderr, .. }) => {
                    eprintln!(
                        "  {entry}/{:?}/{:?} unavailable on this host:\n{stderr}",
                        stage, target
                    );
                }
                Err(e) => panic!("unexpected error compiling {entry}/{stage:?}/{target:?}: {e}"),
            }
        }
    }

    let mut serialised = String::new();
    serialised.push_str("# engine-shader reproducibility golden (ADR-038)\n");
    serialised.push_str(&format!("# SLANGC_PIN = {SLANGC_PIN}\n"));
    for ((stage, entry, target), digest) in &digests {
        serialised.push_str(&format!("{stage}\t{entry}\t{target}\t{digest}\n"));
    }

    if std::env::var("ENGINE_GOLDEN_WRITE").is_ok() {
        std::fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
        std::fs::write(golden_path(), &serialised).unwrap();
        eprintln!("wrote {}", golden_path().display());
        return;
    }

    if !enforce_digests {
        return;
    }

    let golden = match std::fs::read_to_string(golden_path()) {
        Ok(g) => g,
        Err(_) => panic!(
            "missing golden at {} — generate with \
             `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-shader --test reproducibility`",
            golden_path().display()
        ),
    };

    // Match the golden entry-by-entry; ignore lines for targets that
    // weren't available on this host (so a Linux CI runner happily
    // matches the subset it can compile).
    let golden_lines: std::collections::BTreeMap<String, String> = golden
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut it = l.split('\t');
            let stage = it.next()?;
            let entry = it.next()?;
            let target = it.next()?;
            let digest = it.next()?;
            Some((format!("{stage}\t{entry}\t{target}"), digest.to_string()))
        })
        .collect();

    for ((stage, entry, target), digest) in &digests {
        let key = format!("{stage}\t{entry}\t{target}");
        match golden_lines.get(&key) {
            Some(expected) if expected == digest => {}
            Some(expected) => panic!(
                "digest mismatch for {key}:\n  expected: {expected}\n  found:    {digest}\n\
                 Re-generate with `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-shader --test reproducibility`."
            ),
            None => panic!(
                "no golden entry for {key} — re-generate the golden via \
                 `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-shader --test reproducibility`"
            ),
        }
    }
}
