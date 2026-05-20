//! The content-addressed blob store.
//!
//! A [`ContentStore`] maps a [`ContentHash`] to the bytes that hash to it.
//! Because the key *is* the content, inserting the same bytes twice is a
//! no-op — this is where the pipeline's deduplication and cache-hit behaviour
//! come from (spec IV.8). The foundation store is in-memory; a disk-backed
//! cache layers on later without changing this API.

use crate::hash::ContentHash;
use engine_core::collections::HashMap;
use engine_platform::mmap::MmapRo;
use std::ops::Range;
use std::sync::Arc;

/// Where the bytes for one pak entry actually live.
///
/// Pak archives produced by [`Pak::builder`](crate::pak::Pak::builder) hold
/// [`BlobSource::Owned`] blobs — the bytes sit in a [`Vec<u8>`] inside the
/// pak. Pak archives opened via
/// [`Pak::open_mmap`](crate::pak::Pak::open_mmap) hold [`BlobSource::Mapped`]
/// blobs — every entry borrows a sub-range of one [`Arc<MmapRo>`] without
/// copying. The variant is invisible to callers: both produce the same
/// `&[u8]` from [`BlobSource::as_bytes`].
pub enum BlobSource {
    /// Bytes owned by the pak. Produced by builder / deserialization paths.
    Owned(Vec<u8>),
    /// Bytes borrowed from a shared memory-mapped file (ADR-029). The
    /// [`MmapRo`] is co-owned by every [`BlobSource::Mapped`] inside the
    /// same pak so the kernel mapping outlives every borrow.
    Mapped {
        /// Shared handle to the kernel mapping.
        mmap: Arc<MmapRo>,
        /// Sub-range of `mmap.as_bytes()` that this blob covers.
        range: Range<usize>,
    },
}

impl BlobSource {
    /// Returns the blob bytes. Zero-copy for both variants.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            BlobSource::Owned(v) => v.as_slice(),
            BlobSource::Mapped { mmap, range } => &mmap.as_bytes()[range.clone()],
        }
    }

    /// Convenience: the blob's length in bytes.
    pub fn len(&self) -> usize {
        match self {
            BlobSource::Owned(v) => v.len(),
            BlobSource::Mapped { range, .. } => range.len(),
        }
    }

    /// Convenience: `true` for a zero-length blob.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for BlobSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owned(v) => f.debug_struct("Owned").field("len", &v.len()).finish(),
            Self::Mapped { range, .. } => f
                .debug_struct("Mapped")
                .field("offset", &range.start)
                .field("len", &range.len())
                .finish(),
        }
    }
}

/// An in-memory, content-addressed blob store.
///
/// The blob lookup is the owned Robin Hood [`HashMap`] (ADR-028) with the
/// default [`FastHasher`](engine_core::collections::FastHasher) — content
/// hashes are already random-looking 256-bit values so the fast multiplicative
/// hasher gives the best probe distribution for the smallest cost.
#[derive(Debug, Default)]
pub struct ContentStore {
    blobs: HashMap<ContentHash, Vec<u8>>,
    inserts: u64,
    cache_hits: u64,
}

impl ContentStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts `bytes`, returning their content hash.
    ///
    /// If the content is already present nothing is stored and a cache hit is
    /// recorded — identical bytes deduplicate to one blob.
    pub fn insert(&mut self, bytes: impl Into<Vec<u8>>) -> ContentHash {
        let bytes = bytes.into();
        let hash = ContentHash::of(&bytes);
        self.inserts += 1;
        if self.blobs.contains_key(&hash) {
            self.cache_hits += 1;
        } else {
            self.blobs.insert(hash, bytes);
        }
        hash
    }

    /// Borrows the bytes for `hash`, if present.
    pub fn get(&self, hash: ContentHash) -> Option<&[u8]> {
        self.blobs.get(&hash).map(Vec::as_slice)
    }

    /// Returns `true` if `hash` is stored.
    pub fn contains(&self, hash: ContentHash) -> bool {
        self.blobs.contains_key(&hash)
    }

    /// The number of distinct blobs held.
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    /// Total `insert` calls (including deduplicated ones).
    pub fn inserts(&self) -> u64 {
        self.inserts
    }

    /// Inserts that hit an already-stored blob.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_get() {
        let mut store = ContentStore::new();
        let hash = store.insert(b"mesh-data".to_vec());
        assert_eq!(store.get(hash), Some(&b"mesh-data"[..]));
        assert!(store.contains(hash));
    }

    #[test]
    fn identical_content_deduplicates() {
        let mut store = ContentStore::new();
        let a = store.insert(b"texture".to_vec());
        let b = store.insert(b"texture".to_vec());
        assert_eq!(a, b);
        assert_eq!(store.blob_count(), 1);
        assert_eq!(store.inserts(), 2);
        assert_eq!(store.cache_hits(), 1);
    }

    #[test]
    fn distinct_content_is_kept_separately() {
        let mut store = ContentStore::new();
        store.insert(b"a".to_vec());
        store.insert(b"b".to_vec());
        assert_eq!(store.blob_count(), 2);
        assert_eq!(store.cache_hits(), 0);
    }
}
