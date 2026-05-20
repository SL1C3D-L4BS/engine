//! Oracle for [`Pak::open_mmap`] — the mmap-backed pak loader (ADR-029).
//!
//! Three checks:
//!
//! 1. **Parity** — build a pak in memory with `Pak::builder`, serialize it
//!    to a tempfile, re-open via `Pak::open_mmap`, and confirm every blob
//!    yields identical bytes (verified by re-hashing) and identical
//!    `entry_names` / `hash_of` lookups.
//! 2. **Truncated file** — write a pak then chop the last blob's tail off
//!    and expect [`PakError::Truncated`] or [`PakError::OutOfBounds`].
//!    The mmap *open* itself must not panic and must not let a SIGBUS
//!    escape to the caller.
//! 3. **Out-of-bounds header** — synthesize a pak whose blob header
//!    claims a length past EOF and expect [`PakError::OutOfBounds`].

use engine_asset::{ContentHash, Pak, PakError};

fn tmp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "engine-asset-mmap-{}-{}.pak",
        std::process::id(),
        name
    ))
}

#[test]
fn round_trips_against_in_memory_pak() {
    // 1 000 blobs of randomized contents — keeps the test fast while still
    // crossing the page boundary several times (1 000 × ~32 B ≈ 30 KiB).
    let mut builder = Pak::builder();
    let mut state: u64 = 0xC0DE_FACE_BEEF_CAFE;
    let mut expected: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..1_000u32 {
        let mut bytes = Vec::with_capacity(48);
        for _ in 0..(8 + (i % 32) as usize) {
            state = state
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(i as u64);
            bytes.extend_from_slice(&state.to_le_bytes());
        }
        let name = format!("blob/{:04}.bin", i);
        builder.add(&name, bytes.clone());
        expected.push((name, bytes));
    }
    let original = builder.build();
    let serialized = original.to_bytes();

    let path = tmp_path("round-trip");
    std::fs::write(&path, &serialized).unwrap();

    let mapped = Pak::open_mmap(&path).expect("open_mmap on a freshly-written pak");

    // Every blob's bytes must hash to the same ContentHash both ways.
    for (name, want) in &expected {
        let mapped_bytes = mapped.get(name).expect("entry present after mmap");
        assert_eq!(mapped_bytes, want.as_slice(), "blob bytes drift for {name}");
        assert_eq!(
            ContentHash::of(mapped_bytes),
            ContentHash::of(want),
            "content hash drift for {name}"
        );
    }
    // Entry-name iteration order matches because both implementations
    // store entries in a `BTreeMap<String, _>`.
    let mapped_names: Vec<&str> = mapped.entry_names().collect();
    let in_mem_names: Vec<&str> = original.entry_names().collect();
    assert_eq!(mapped_names, in_mem_names);

    std::fs::remove_file(&path).ok();
}

#[test]
fn truncated_pak_is_rejected_without_sigbus() {
    let mut builder = Pak::builder();
    builder.add("a.bin", b"alpha".to_vec());
    builder.add("b.bin", vec![0u8; 1024]); // last blob; we'll chop its tail
    let pak = builder.build();
    let bytes = pak.to_bytes();

    let path = tmp_path("truncated");
    // Drop the last 32 bytes so the trailing blob's declared length runs
    // past EOF.
    std::fs::write(&path, &bytes[..bytes.len() - 32]).unwrap();

    let err = Pak::open_mmap(&path).expect_err("truncated pak must fail to open");
    // Either Truncated or OutOfBounds is correct — the precise variant
    // depends on whether the missing tail crossed the blob-header
    // boundary or only the blob body. Both are non-SIGBUS observable
    // failure modes, which is the whole point of the explicit length
    // check inside open_mmap.
    assert!(
        matches!(err, PakError::Truncated | PakError::OutOfBounds),
        "expected Truncated or OutOfBounds, got {err:?}"
    );

    std::fs::remove_file(&path).ok();
}

#[test]
fn header_claiming_past_eof_is_rejected() {
    // Synthesize a minimal valid header for one entry, then declare a
    // blob whose length runs past EOF.
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(b"ENGNPAK1"); // magic
    buf.extend_from_slice(&1u32.to_le_bytes()); // version

    // One entry: name=`x`, hash=hash_of(b"hello")
    let hash = ContentHash::of(b"hello");
    buf.extend_from_slice(&1u32.to_le_bytes()); // entry count
    buf.extend_from_slice(&1u32.to_le_bytes()); // name_len
    buf.push(b'x'); // name
    buf.extend_from_slice(hash.as_bytes()); // hash

    // One blob: claim 1 GiB even though we will only write 5 bytes after.
    buf.extend_from_slice(&1u32.to_le_bytes()); // blob count
    buf.extend_from_slice(hash.as_bytes()); // blob hash
    buf.extend_from_slice(&(1u32 << 30).to_le_bytes()); // blob_len = 1 GiB
    buf.extend_from_slice(b"hello"); // only 5 bytes of body

    let path = tmp_path("oob-header");
    std::fs::write(&path, &buf).unwrap();

    let err = Pak::open_mmap(&path).expect_err("OOB-declared blob must be rejected");
    assert!(
        matches!(err, PakError::OutOfBounds),
        "expected OutOfBounds, got {err:?}"
    );

    std::fs::remove_file(&path).ok();
}
