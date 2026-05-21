//! Compiled shader artefact + on-disk bundle codec.
//!
//! An [`Artifact`] is the result of one `slangc` invocation: bytes for
//! one target, reflection JSON, plus the BLAKE3 digest of the bytes.
//! A [`Bundle`] groups artefacts for all four targets of a single
//! entry point so the asset pak can ship one record per shader.
//!
//! On-disk format (little-endian, owned — no serde):
//!
//! ```text
//! Bundle:
//!   magic     "SHDR"             // 4 B
//!   version   u16                // currently 1
//!   stage_tag u8                 // Stage::tag
//!   entry_len u16                // bytes
//!   entry     [u8; entry_len]
//!   artifacts u8                 // count (1..=4)
//!     artefact_0 ... artefact_n
//!
//! Artifact:
//!   target_tag    u8
//!   bytes_len     u32
//!   bytes         [u8; bytes_len]
//!   reflection_len u32
//!   reflection    [u8; reflection_len]
//!   digest        [u8; 32]      // BLAKE3 over `bytes`
//! ```

use crate::target::{Stage, Target};
use blake3::Hasher;
use engine_asset::{Asset, AssetError};

/// One compiled shader output for a single target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    /// Which backend produced these bytes.
    pub target: Target,
    /// Compiled bytes (SPIR-V / DXIL binary; WGSL / MSL source).
    pub bytes: Vec<u8>,
    /// `slangc` reflection JSON. Owned; not interpreted by this crate.
    pub reflection: Vec<u8>,
    /// BLAKE3 digest of `bytes`. The reproducibility golden compares
    /// per-target digests across runs and across architectures
    /// (ADR-038).
    pub digest: [u8; 32],
}

impl Artifact {
    /// Constructs an artefact and hashes its bytes.
    pub fn new(target: Target, bytes: Vec<u8>, reflection: Vec<u8>) -> Self {
        let mut h = Hasher::new();
        h.update(&bytes);
        let digest = *h.finalize().as_bytes();
        Self {
            target,
            bytes,
            reflection,
            digest,
        }
    }

    /// Hex-encoded BLAKE3 digest (lowercase). The committed
    /// reproducibility golden uses this form.
    pub fn digest_hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in &self.digest {
            out.push_str(&format!("{b:02x}"));
        }
        out
    }
}

/// A complete shader: one entry point, one stage, up to four targets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bundle {
    /// Entry-point function name (`-entry` flag value to slangc).
    pub entry: String,
    /// Pipeline stage.
    pub stage: Stage,
    /// One artefact per target compiled. Stable sort by
    /// `Target::tag()` so the digest concatenation is reproducible.
    pub artifacts: Vec<Artifact>,
}

impl Bundle {
    /// Constructs a bundle. Artifacts are sorted by `Target::tag()` so
    /// the BLAKE3 digest [`Self::bundle_digest`] is invariant across
    /// the order the caller compiled in.
    pub fn new(entry: impl Into<String>, stage: Stage, mut artifacts: Vec<Artifact>) -> Self {
        artifacts.sort_by_key(|a| a.target.tag());
        Self {
            entry: entry.into(),
            stage,
            artifacts,
        }
    }

    /// BLAKE3 digest over the concatenated per-artefact digests.
    /// Identical bytes across two compilations imply identical
    /// digests; the reproducibility oracle keys on this.
    pub fn bundle_digest(&self) -> [u8; 32] {
        let mut h = Hasher::new();
        h.update(self.entry.as_bytes());
        h.update(&[self.stage.tag()]);
        for a in &self.artifacts {
            h.update(&[a.target.tag()]);
            h.update(&a.digest);
        }
        *h.finalize().as_bytes()
    }

    /// Lookup an artefact by target.
    pub fn target(&self, target: Target) -> Option<&Artifact> {
        self.artifacts.iter().find(|a| a.target == target)
    }
}

/// On-disk magic for [`Bundle`] records.
pub const BUNDLE_MAGIC: [u8; 4] = *b"SHDR";

/// Current on-disk format version.
pub const BUNDLE_VERSION: u16 = 1;

/// Encodes `bundle` to bytes. Format documented at module top.
pub fn encode(bundle: &Bundle) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&BUNDLE_MAGIC);
    out.extend_from_slice(&BUNDLE_VERSION.to_le_bytes());
    out.push(bundle.stage.tag());
    let entry = bundle.entry.as_bytes();
    out.extend_from_slice(&(entry.len() as u16).to_le_bytes());
    out.extend_from_slice(entry);
    out.push(bundle.artifacts.len() as u8);
    for a in &bundle.artifacts {
        out.push(a.target.tag());
        out.extend_from_slice(&(a.bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&a.bytes);
        out.extend_from_slice(&(a.reflection.len() as u32).to_le_bytes());
        out.extend_from_slice(&a.reflection);
        out.extend_from_slice(&a.digest);
    }
    out
}

/// Why a [`Bundle`] could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Header magic mismatch.
    BadMagic,
    /// Unknown on-disk format version.
    BadVersion(u16),
    /// Stage tag does not map to a known stage.
    BadStage(u8),
    /// Target tag does not map to a known target.
    BadTarget(u8),
    /// Truncated input.
    Truncated,
    /// Stored digest disagreed with re-hashed bytes — data corruption.
    DigestMismatch,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "shader bundle: bad magic"),
            Self::BadVersion(v) => write!(f, "shader bundle: unknown version {v}"),
            Self::BadStage(b) => write!(f, "shader bundle: unknown stage tag {b}"),
            Self::BadTarget(b) => write!(f, "shader bundle: unknown target tag {b}"),
            Self::Truncated => write!(f, "shader bundle: truncated input"),
            Self::DigestMismatch => write!(f, "shader bundle: digest mismatch on stored bytes"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes the bytes produced by [`encode`].
pub fn decode(bytes: &[u8]) -> Result<Bundle, DecodeError> {
    let mut cur = Cursor::new(bytes);
    let magic = cur.take(4)?;
    if magic != BUNDLE_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = cur.u16()?;
    if version != BUNDLE_VERSION {
        return Err(DecodeError::BadVersion(version));
    }
    let stage = Stage::from_tag(cur.u8()?).ok_or(DecodeError::BadStage(0))?;
    let entry_len = cur.u16()? as usize;
    let entry_bytes = cur.slice(entry_len)?;
    let entry = String::from_utf8_lossy(entry_bytes).into_owned();
    let count = cur.u8()? as usize;
    let mut artifacts = Vec::with_capacity(count);
    for _ in 0..count {
        let target_tag = cur.u8()?;
        let target = Target::from_tag(target_tag).ok_or(DecodeError::BadTarget(target_tag))?;
        let blen = cur.u32()? as usize;
        let bytes = cur.slice(blen)?.to_vec();
        let rlen = cur.u32()? as usize;
        let reflection = cur.slice(rlen)?.to_vec();
        let stored_digest = cur.take(32)?;
        let mut h = Hasher::new();
        h.update(&bytes);
        let actual = *h.finalize().as_bytes();
        if stored_digest != actual {
            return Err(DecodeError::DigestMismatch);
        }
        artifacts.push(Artifact {
            target,
            bytes,
            reflection,
            digest: actual,
        });
    }
    Ok(Bundle {
        entry,
        stage,
        artifacts,
    })
}

impl Asset for Bundle {
    fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
        decode(bytes).map_err(|e| AssetError::Decode(e.to_string()))
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn slice(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        self.take(n)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}
