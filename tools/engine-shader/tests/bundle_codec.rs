//! Bundle on-disk codec oracle (PR 4, ADR-037).
//!
//! No slangc required — hand-built artifacts exercise the encoder /
//! decoder + the `impl Asset` decode path + the bundle digest
//! invariants.

use engine_shader::artifact::{Artifact, Bundle, DecodeError, decode, encode};
use engine_shader::target::{Stage, Target};

fn art(target: Target, bytes: &[u8]) -> Artifact {
    Artifact::new(target, bytes.to_vec(), format!("{target:?}").into_bytes())
}

#[test]
fn roundtrip_one_target() {
    let b = Bundle::new(
        "vs_main",
        Stage::Vertex,
        vec![art(Target::SpirV, &[0x07, 0x00, 0x02, 0x03])],
    );
    let enc = encode(&b);
    let dec = decode(&enc).expect("decode");
    assert_eq!(dec, b);
}

#[test]
fn roundtrip_four_targets() {
    let b = Bundle::new(
        "fs_main",
        Stage::Fragment,
        vec![
            art(Target::SpirV, b"spv bytes"),
            art(Target::Wgsl, b"wgsl source"),
            art(Target::Dxil, b"dxil bytes"),
            art(Target::Msl, b"msl source"),
        ],
    );
    assert_eq!(b.artifacts.len(), 4);
    let enc = encode(&b);
    let dec = decode(&enc).expect("decode");
    assert_eq!(dec, b);
    // Lookup by target.
    assert_eq!(dec.target(Target::Wgsl).unwrap().bytes, b"wgsl source");
}

#[test]
fn bundle_digest_invariant_under_input_order() {
    let a = Bundle::new(
        "main",
        Stage::Compute,
        vec![
            art(Target::Msl, b"msl"),
            art(Target::SpirV, b"spv"),
            art(Target::Wgsl, b"wgsl"),
        ],
    );
    let b = Bundle::new(
        "main",
        Stage::Compute,
        vec![
            art(Target::Wgsl, b"wgsl"),
            art(Target::Msl, b"msl"),
            art(Target::SpirV, b"spv"),
        ],
    );
    assert_eq!(a.bundle_digest(), b.bundle_digest());
}

#[test]
fn bundle_digest_changes_with_bytes() {
    let a = Bundle::new("main", Stage::Vertex, vec![art(Target::SpirV, b"one")]);
    let b = Bundle::new("main", Stage::Vertex, vec![art(Target::SpirV, b"two")]);
    assert_ne!(a.bundle_digest(), b.bundle_digest());
}

#[test]
fn decode_rejects_bad_magic() {
    let mut enc = encode(&Bundle::new(
        "x",
        Stage::Vertex,
        vec![art(Target::SpirV, b"spv")],
    ));
    enc[0] = b'B';
    assert_eq!(decode(&enc), Err(DecodeError::BadMagic));
}

#[test]
fn decode_rejects_truncation() {
    let enc = encode(&Bundle::new(
        "x",
        Stage::Vertex,
        vec![art(Target::SpirV, b"spv")],
    ));
    let truncated = &enc[..enc.len() - 5];
    assert_eq!(decode(truncated), Err(DecodeError::Truncated));
}

#[test]
fn decode_detects_digest_corruption() {
    let mut enc = encode(&Bundle::new(
        "x",
        Stage::Vertex,
        vec![art(Target::SpirV, &[1, 2, 3, 4])],
    ));
    // Flip a byte inside the artefact `bytes` payload (after
    // header + entry + count + target_tag + bytes_len). Don't flip
    // the digest — we want to catch bytes-vs-digest divergence.
    let payload_offset = enc.len() - (32 + 4 + 7/*reflection len + body*/);
    enc[payload_offset] ^= 0xFF;
    assert_eq!(decode(&enc), Err(DecodeError::DigestMismatch));
}

#[test]
fn asset_impl_decodes() {
    use engine_asset::Asset;
    let b = Bundle::new("main", Stage::Compute, vec![art(Target::SpirV, &[9, 9, 9])]);
    let enc = encode(&b);
    let dec = <Bundle as Asset>::decode(&enc).expect("Asset::decode");
    assert_eq!(dec, b);
}
