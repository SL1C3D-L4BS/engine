//! Cross-arch byte-equal compile oracle (ADR-034).
//!
//! Compiles every fixture in [`common::corpus`], serialises the optimised
//! IR into a stable text form, takes a BLAKE3 digest over the concatenated
//! result, and compares against the committed golden under
//! `tests/goldens/sli-compile.golden`. The Phase-3 determinism matrix
//! runs this on both x86-64 and aarch64 against the same golden — two
//! architectures agreeing with one digest proves cross-arch byte-equality
//! transitively (ADR-013 pattern).
//!
//! To regenerate the golden after an intentional change:
//!
//! ```sh
//! ENGINE_GOLDEN_WRITE=1 cargo test -p engine-script --test compile_parity
//! ```

mod common;

use blake3::Hasher;
use engine_script::{Compiler, Source, SourceMap};
use std::fs;
use std::path::PathBuf;

#[test]
fn corpus_digest_matches_golden() {
    let mut sm = SourceMap::new();
    let mut hasher = Hasher::new();
    let mut combined_ir = String::new();
    for fix in common::corpus() {
        let src = Source::new(format!("{}.sli", fix.name), fix.source);
        let id = sm.add(src);
        let compiled = Compiler::new()
            .compile(id, sm.get(id))
            .expect("compilation");
        assert!(
            !compiled.diagnostics.has_errors(),
            "{} produced diagnostics: {:?}",
            fix.name,
            compiled
                .diagnostics
                .all()
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>(),
        );
        let serialised = engine_script::ir::serialise(&compiled.ir);
        combined_ir.push_str("=== ");
        combined_ir.push_str(fix.name);
        combined_ir.push_str(" ===\n");
        combined_ir.push_str(&serialised);
        hasher.update(b"=== ");
        hasher.update(fix.name.as_bytes());
        hasher.update(b" ===\n");
        hasher.update(serialised.as_bytes());
    }
    let digest = hasher.finalize();
    let digest_hex = digest.to_hex().to_string();

    let golden_path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join("sli-compile.golden");

    if std::env::var("ENGINE_GOLDEN_WRITE").is_ok() {
        fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        fs::write(&golden_path, format!("{digest_hex}\n")).unwrap();
        eprintln!("wrote golden {}", golden_path.display());
        return;
    }

    let expected = fs::read_to_string(&golden_path)
        .expect("missing golden — regenerate with ENGINE_GOLDEN_WRITE=1");
    let expected = expected.trim();
    assert_eq!(
        digest_hex, expected,
        "sli-compile digest drift\n  expected: {expected}\n  got     : {digest_hex}\nIR text:\n{combined_ir}",
    );
}
