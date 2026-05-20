//! Pak archives and overlay mounting.
//!
//! A [`Pak`] is a content-addressed archive: logical names map to
//! [`ContentHash`]es, and the hashed blobs travel with it. The serialized form
//! is deterministic — entries and blobs are written in sorted order — so the
//! same inputs always produce byte-identical pak files (spec IV.8).
//!
//! [`PakSet`] implements the Live Ops overlay model: a base pak plus update
//! paks, resolved newest-first. A broken asset can be kill-switched by name
//! without shipping a patch — game code never learns which pak an asset came
//! from.

use crate::hash::ContentHash;
use std::collections::{BTreeMap, BTreeSet};

const MAGIC: &[u8; 8] = b"ENGNPAK1";
const FORMAT_VERSION: u32 = 1;

/// An error encountered while decoding a [`Pak`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PakError {
    /// The magic number did not match — not a pak file.
    BadMagic,
    /// The format version is newer than this build understands.
    UnsupportedVersion(u32),
    /// The data ended before a complete pak could be read.
    Truncated,
    /// A blob's bytes did not match its declared content hash.
    IntegrityFailure,
}

/// A content-addressed asset archive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pak {
    entries: BTreeMap<String, ContentHash>,
    blobs: BTreeMap<ContentHash, Vec<u8>>,
}

/// Accumulates entries into a [`Pak`].
#[derive(Debug, Default)]
pub struct PakBuilder {
    pak: Pak,
}

impl PakBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an entry, content-addressing its bytes. Returns the content hash.
    ///
    /// Adding the same name again replaces the entry; identical bytes under
    /// different names share a single blob.
    pub fn add(&mut self, name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> ContentHash {
        let bytes = bytes.into();
        let hash = ContentHash::of(&bytes);
        self.pak.entries.insert(name.into(), hash);
        self.pak.blobs.entry(hash).or_insert(bytes);
        hash
    }

    /// Finishes building.
    pub fn build(self) -> Pak {
        self.pak
    }
}

impl Pak {
    /// Starts building a new pak.
    pub fn builder() -> PakBuilder {
        PakBuilder::new()
    }

    /// Borrows the bytes of the entry `name`.
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let hash = self.entries.get(name)?;
        self.blobs.get(hash).map(Vec::as_slice)
    }

    /// The content hash of the entry `name`.
    pub fn hash_of(&self, name: &str) -> Option<ContentHash> {
        self.entries.get(name).copied()
    }

    /// Returns `true` if `name` is present.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Iterates entry names in sorted order.
    pub fn entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The number of named entries.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// The number of distinct blobs (entries may share blobs).
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    /// Serializes the pak to its deterministic binary form.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());

        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (name, hash) in &self.entries {
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(hash.as_bytes());
        }

        out.extend_from_slice(&(self.blobs.len() as u32).to_le_bytes());
        for (hash, blob) in &self.blobs {
            out.extend_from_slice(hash.as_bytes());
            out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            out.extend_from_slice(blob);
        }
        out
    }

    /// Decodes a pak from its binary form, verifying every blob's integrity.
    pub fn from_bytes(data: &[u8]) -> Result<Self, PakError> {
        let mut reader = Reader::new(data);
        if reader.take(8).ok_or(PakError::Truncated)? != MAGIC {
            return Err(PakError::BadMagic);
        }
        let version = reader.u32().ok_or(PakError::Truncated)?;
        if version != FORMAT_VERSION {
            return Err(PakError::UnsupportedVersion(version));
        }

        let mut entries = BTreeMap::new();
        let entry_count = reader.u32().ok_or(PakError::Truncated)?;
        for _ in 0..entry_count {
            let name_len = reader.u32().ok_or(PakError::Truncated)? as usize;
            let name = reader.take(name_len).ok_or(PakError::Truncated)?;
            let name = std::str::from_utf8(name).map_err(|_| PakError::Truncated)?;
            let hash = reader.hash().ok_or(PakError::Truncated)?;
            entries.insert(name.to_string(), hash);
        }

        let mut blobs = BTreeMap::new();
        let blob_count = reader.u32().ok_or(PakError::Truncated)?;
        for _ in 0..blob_count {
            let hash = reader.hash().ok_or(PakError::Truncated)?;
            let blob_len = reader.u32().ok_or(PakError::Truncated)? as usize;
            let blob = reader.take(blob_len).ok_or(PakError::Truncated)?;
            if ContentHash::of(blob) != hash {
                return Err(PakError::IntegrityFailure);
            }
            blobs.insert(hash, blob.to_vec());
        }

        Ok(Self { entries, blobs })
    }
}

/// A stack of mounted [`Pak`]s resolved newest-first, with per-name
/// kill-switching (the Live Ops overlay model — spec IV.8 / ADR-008).
#[derive(Debug, Default)]
pub struct PakSet {
    /// Mount order; later entries are newer and take precedence.
    paks: Vec<Pak>,
    disabled: BTreeSet<String>,
}

impl PakSet {
    /// Creates an empty pak set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts `pak` as the newest overlay.
    pub fn mount(&mut self, pak: Pak) {
        self.paks.push(pak);
    }

    /// Resolves `name` against the mounted paks, newest first.
    ///
    /// Returns `None` if no pak provides the name or it has been kill-switched.
    pub fn resolve(&self, name: &str) -> Option<&[u8]> {
        if self.disabled.contains(name) {
            return None;
        }
        self.paks.iter().rev().find_map(|pak| pak.get(name))
    }

    /// Kill-switches `name` — it resolves to `None` until re-enabled, without
    /// touching any pak.
    pub fn disable(&mut self, name: impl Into<String>) {
        self.disabled.insert(name.into());
    }

    /// Lifts a kill-switch.
    pub fn enable(&mut self, name: &str) {
        self.disabled.remove(name);
    }

    /// Returns `true` if `name` is currently kill-switched.
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.contains(name)
    }

    /// The number of mounted paks.
    pub fn mounted_count(&self) -> usize {
        self.paks.len()
    }
}

/// Minimal forward cursor over a byte slice.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn hash(&mut self) -> Option<ContentHash> {
        Some(ContentHash::from_bytes(self.take(32)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pak() -> Pak {
        let mut b = Pak::builder();
        b.add("shaders/lit.spv", b"spirv-bytes".to_vec());
        b.add("meshes/cube.mesh", b"mesh-bytes".to_vec());
        b.add("alias.spv", b"spirv-bytes".to_vec()); // shares a blob
        b.build()
    }

    #[test]
    fn entries_and_shared_blobs() {
        let pak = sample_pak();
        assert_eq!(pak.entry_count(), 3);
        assert_eq!(pak.blob_count(), 2); // "spirv-bytes" stored once
        assert_eq!(pak.get("shaders/lit.spv"), Some(&b"spirv-bytes"[..]));
        assert_eq!(pak.hash_of("alias.spv"), pak.hash_of("shaders/lit.spv"));
    }

    #[test]
    fn serialization_is_deterministic_and_round_trips() {
        let pak = sample_pak();
        let bytes = pak.to_bytes();
        assert_eq!(bytes, sample_pak().to_bytes()); // deterministic
        let decoded = Pak::from_bytes(&bytes).expect("valid pak");
        assert_eq!(decoded, pak);
    }

    #[test]
    fn decoding_rejects_corruption() {
        assert_eq!(Pak::from_bytes(b"not a pak"), Err(PakError::BadMagic));
        assert_eq!(Pak::from_bytes(b"ENGNPAK1"), Err(PakError::Truncated));

        let mut bytes = sample_pak().to_bytes();
        // Flip a byte inside the last blob: integrity check must catch it.
        *bytes.last_mut().unwrap() ^= 0xff;
        assert_eq!(Pak::from_bytes(&bytes), Err(PakError::IntegrityFailure));
    }

    #[test]
    fn overlay_resolves_newest_first() {
        let mut base = Pak::builder();
        base.add("config.ron", b"v1".to_vec());
        base.add("logo.tex", b"original".to_vec());

        let mut patch = Pak::builder();
        patch.add("config.ron", b"v2".to_vec());

        let mut set = PakSet::new();
        set.mount(base.build());
        set.mount(patch.build());

        assert_eq!(set.resolve("config.ron"), Some(&b"v2"[..])); // patch wins
        assert_eq!(set.resolve("logo.tex"), Some(&b"original"[..])); // base shows through
        assert_eq!(set.mounted_count(), 2);
    }

    #[test]
    fn kill_switch_hides_an_asset() {
        let mut pak = Pak::builder();
        pak.add("broken.tex", b"crash".to_vec());
        let mut set = PakSet::new();
        set.mount(pak.build());

        assert!(set.resolve("broken.tex").is_some());
        set.disable("broken.tex");
        assert!(set.is_disabled("broken.tex"));
        assert_eq!(set.resolve("broken.tex"), None);
        set.enable("broken.tex");
        assert!(set.resolve("broken.tex").is_some());
    }
}
