//! Target / Stage enum oracles (PR 4, ADR-037).
//!
//! No slangc required. Pins on-disk tags, slangc flag strings, and
//! the canonical iteration order. Bumping any of these is a
//! breaking change to the asset format.

use engine_shader::target::{Stage, Target};

#[test]
fn target_tags_are_stable() {
    assert_eq!(Target::SpirV.tag(), 1);
    assert_eq!(Target::Wgsl.tag(), 2);
    assert_eq!(Target::Dxil.tag(), 3);
    assert_eq!(Target::Msl.tag(), 4);
    for t in Target::all() {
        assert_eq!(Target::from_tag(t.tag()), Some(*t));
    }
    assert_eq!(Target::from_tag(0), None);
    assert_eq!(Target::from_tag(255), None);
}

#[test]
fn stage_tags_are_stable() {
    assert_eq!(Stage::Vertex.tag(), 1);
    assert_eq!(Stage::Fragment.tag(), 2);
    assert_eq!(Stage::Compute.tag(), 3);
    for s in [Stage::Vertex, Stage::Fragment, Stage::Compute] {
        assert_eq!(Stage::from_tag(s.tag()), Some(s));
    }
    assert_eq!(Stage::from_tag(0), None);
}

#[test]
fn slangc_flags_match_doc() {
    assert_eq!(Target::SpirV.slangc_flag(), "spirv");
    assert_eq!(Target::Wgsl.slangc_flag(), "wgsl");
    assert_eq!(Target::Dxil.slangc_flag(), "dxil");
    assert_eq!(Target::Msl.slangc_flag(), "metal");
    assert_eq!(Stage::Vertex.slangc_flag(), "vertex");
    assert_eq!(Stage::Fragment.slangc_flag(), "fragment");
    assert_eq!(Stage::Compute.slangc_flag(), "compute");
}

#[test]
fn extension_routing() {
    assert_eq!(Target::SpirV.extension(), "spv");
    assert_eq!(Target::Wgsl.extension(), "wgsl");
    assert_eq!(Target::Dxil.extension(), "dxil");
    assert_eq!(Target::Msl.extension(), "metal");
}

#[test]
fn iteration_order_matches_tag_order() {
    let mut last = 0u8;
    for t in Target::all() {
        assert!(
            t.tag() > last,
            "Target::all() must be sorted by tag for the bundle digest to be reproducible"
        );
        last = t.tag();
    }
}

#[test]
fn is_binary_classification() {
    assert!(Target::SpirV.is_binary());
    assert!(Target::Dxil.is_binary());
    assert!(!Target::Wgsl.is_binary());
    assert!(!Target::Msl.is_binary());
}
