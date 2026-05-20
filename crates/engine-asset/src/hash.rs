//! Content addressing.
//!
//! Every compiled asset is keyed by the SHA-256 of its bytes (spec IV.8). This
//! is what makes the pipeline deterministic, deduplicating, and
//! delta-patchable: identical bytes always produce an identical [`ContentHash`].

use sha2::{Digest, Sha256};
use std::fmt;

/// The SHA-256 content hash of a byte sequence.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Computes the content hash of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }

    /// The raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Builds a hash from a raw 32-byte digest.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The lowercase hex encoding (64 characters).
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(64);
        for byte in self.0 {
            s.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
            s.push(char::from_digit((byte & 0xf) as u32, 16).unwrap());
        }
        s
    }

    /// Parses a 64-character hex string, or returns `None` if malformed.
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        let chars: Vec<char> = s.chars().collect();
        for (i, byte) in bytes.iter_mut().enumerate() {
            let hi = chars[i * 2].to_digit(16)?;
            let lo = chars[i * 2 + 1].to_digit(16)?;
            *byte = ((hi << 4) | lo) as u8;
        }
        Some(Self(bytes))
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The four-character prefix matches the editor's hash display.
        write!(f, "ContentHash({}…)", &self.to_hex()[..8])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_bytes_hash_identically() {
        assert_eq!(ContentHash::of(b"forge"), ContentHash::of(b"forge"));
        assert_ne!(ContentHash::of(b"forge"), ContentHash::of(b"Forge"));
    }

    #[test]
    fn hex_round_trips() {
        let h = ContentHash::of(b"deterministic");
        assert_eq!(h.to_hex().len(), 64);
        assert_eq!(ContentHash::from_hex(&h.to_hex()), Some(h));
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert!(ContentHash::from_hex("tooshort").is_none());
        assert!(ContentHash::from_hex(&"g".repeat(64)).is_none());
    }

    #[test]
    fn known_vector() {
        // SHA-256 of the empty input.
        assert_eq!(
            ContentHash::of(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
