//! The content-addressed blob store.
//!
//! A [`ContentStore`] maps a [`ContentHash`] to the bytes that hash to it.
//! Because the key *is* the content, inserting the same bytes twice is a
//! no-op — this is where the pipeline's deduplication and cache-hit behaviour
//! come from (spec IV.8). The foundation store is in-memory; a disk-backed
//! cache layers on later without changing this API.

use crate::hash::ContentHash;
use std::collections::HashMap;

/// An in-memory, content-addressed blob store.
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
        match self.blobs.entry(hash) {
            std::collections::hash_map::Entry::Occupied(_) => self.cache_hits += 1,
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(bytes);
            }
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
