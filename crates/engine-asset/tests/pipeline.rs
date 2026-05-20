//! Oracle for `engine-asset`: the three pipeline contracts the spec (IV.8)
//! relies on — content-address reproducibility, signed-pak verification, and
//! overlay/kill-switch resolution.

use engine_asset::{
    Asset, AssetError, AssetServer, ContentHash, ContentStore, Pak, PakSet, PakSigner, verify,
};

/// Identical bytes must always produce an identical content hash, on any run
/// and any machine — this is what makes the cache and delta-patching sound.
#[test]
fn content_addressing_is_reproducible_and_deduplicating() {
    let payload = b"the same texel bytes";
    assert_eq!(ContentHash::of(payload), ContentHash::of(payload));
    // Distinct content addresses to distinct keys.
    assert_ne!(ContentHash::of(payload), ContentHash::of(b"other bytes"));

    let mut store = ContentStore::new();
    let h1 = store.insert(payload.to_vec());
    let h2 = store.insert(payload.to_vec()); // same bytes again
    assert_eq!(h1, h2);
    assert_eq!(store.blob_count(), 1); // stored once
    assert_eq!(store.cache_hits(), 1); // second insert was a hit
    assert_eq!(store.get(h1), Some(&payload[..]));
}

/// A content hash survives a hex round-trip — the on-disk/manifest form.
#[test]
fn content_hash_hex_round_trips() {
    let hash = ContentHash::of(b"manifest entry");
    let hex = hash.to_hex();
    assert_eq!(hex.len(), 64);
    assert_eq!(ContentHash::from_hex(&hex), Some(hash));
    assert_eq!(ContentHash::from_hex("not hex"), None);
}

/// Signing a pak and verifying it must round-trip; a tampered pak must fail.
#[test]
fn pak_signing_round_trips_and_detects_tampering() {
    let signer = PakSigner::from_seed(&[42u8; 32]);

    let mut builder = Pak::builder();
    builder.add("levels/intro.scn", b"scene-data".to_vec());
    let pak = builder.build();

    let signature = signer.sign(&pak);
    assert!(verify(&pak, &signature, &signer.public_key()));

    // A pak that decodes the same but carries different content.
    let mut tampered = Pak::builder();
    tampered.add("levels/intro.scn", b"malicious-data".to_vec());
    assert!(!verify(&tampered.build(), &signature, &signer.public_key()));
}

/// Newest pak wins; a kill-switch hides an asset across the whole set.
#[test]
fn overlay_resolves_newest_first_with_kill_switch() {
    let mut base = Pak::builder();
    base.add("ui/theme.ron", b"base-theme".to_vec());
    base.add("ui/broken.tex", b"corrupt".to_vec());

    let mut update = Pak::builder();
    update.add("ui/theme.ron", b"hotfix-theme".to_vec());

    let mut set = PakSet::new();
    set.mount(base.build());
    set.mount(update.build());

    assert_eq!(set.resolve("ui/theme.ron"), Some(&b"hotfix-theme"[..]));
    assert_eq!(set.resolve("ui/broken.tex"), Some(&b"corrupt"[..]));

    set.disable("ui/broken.tex");
    assert_eq!(set.resolve("ui/broken.tex"), None);
}

/// A string asset, used to drive the server end to end.
struct Text(String);

impl Asset for Text {
    fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
        std::str::from_utf8(bytes)
            .map(|s| Text(s.to_string()))
            .map_err(|e| AssetError::Decode(e.to_string()))
    }
}

/// The server resolves through the overlay and hot-reloads from a mounted
/// update pak — the end-to-end Live Ops path.
#[test]
fn asset_server_loads_and_hot_reloads_through_the_overlay() {
    let mut base = Pak::builder();
    base.add("greeting", b"hello".to_vec());

    let mut set = PakSet::new();
    set.mount(base.build());
    let mut server = AssetServer::with_source(set);

    let handle = server.load::<Text>("greeting").unwrap();
    assert_eq!(handle.get().0, "hello");

    let mut patch = Pak::builder();
    patch.add("greeting", b"hello world".to_vec());
    server.source_mut().mount(patch.build());
    server.reload("greeting").unwrap();

    assert_eq!(handle.get().0, "hello world");
}
