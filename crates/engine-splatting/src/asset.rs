//! ESPL asset format encode/decode (ADR-078).
//!
//! 24-byte deterministic header + length-prefixed sections per
//! ADR-078 §1/§2. Mirrors the EMSH/EMAT pattern from ADR-061.
//!
//! ```text
//! Offset Size  Field
//! 0      4     magic = "ESPL"
//! 4      2     version (u16 LE; v1 = 1)
//! 6      2     flags  (u16; bit 0 = has_sh; bit 1 = compressed (reserved))
//! 8      4     splat_count (u32 LE)
//! 12     4     payload_bytes (u32 LE)
//! 16     8     BLAKE3 digest of payload (first 8 bytes; pak uses 32)
//! ```

use crate::cloud::{SH_COEFFS_PER_CHANNEL, SplatCloud, SplatCloudBuilder};
use engine_math::{Quat, Vec3};

/// On-disk magic for ESPL files.
pub const MAGIC: [u8; 4] = *b"ESPL";

/// Current ESPL format version.
pub const VERSION: u16 = 1;

/// On-disk header size.
pub const HEADER_BYTES: usize = 24;

/// Per-splat bytes without SH (Vec3 + Vec3 + Quat + Vec3 + f32 = 56).
pub const BYTES_PER_SPLAT_NO_SH: usize = 12 + 12 + 16 + 12 + 4;

/// Per-splat SH-section bytes (27 × 4).
pub const BYTES_PER_SPLAT_SH: usize = SH_COEFFS_PER_CHANNEL * 3 * 4;

/// Flag bit: payload includes the 27-coef SH per splat.
pub const FLAG_HAS_SH: u16 = 0b0000_0000_0000_0001;

/// Compact descriptor returned by [`decode_meta`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplatCloudMeta {
    /// Number of splats encoded in the file.
    pub splat_count: u32,
    /// Whether the payload includes the SH section.
    pub has_sh: bool,
    /// Full 32-byte BLAKE3 digest of the payload (vs. the truncated
    /// 8 bytes stored in the header).
    pub payload_digest: [u8; 32],
}

/// Encode a [`SplatCloud`] to its on-disk byte representation.
pub fn encode(cloud: &SplatCloud) -> Vec<u8> {
    let count = cloud.len() as u32;
    let has_sh = cloud.sh().is_some();
    let per_splat = BYTES_PER_SPLAT_NO_SH + if has_sh { BYTES_PER_SPLAT_SH } else { 0 };
    let payload_bytes = per_splat * cloud.len();
    let mut payload = Vec::with_capacity(payload_bytes);

    // Section 0: positions
    for p in cloud.position() {
        payload.extend_from_slice(&p.x.to_le_bytes());
        payload.extend_from_slice(&p.y.to_le_bytes());
        payload.extend_from_slice(&p.z.to_le_bytes());
    }
    // Section 1: scales (log-space)
    for s in cloud.scale() {
        payload.extend_from_slice(&s.x.to_le_bytes());
        payload.extend_from_slice(&s.y.to_le_bytes());
        payload.extend_from_slice(&s.z.to_le_bytes());
    }
    // Section 2: rotations (Quat = 4 × f32)
    for q in cloud.rotation() {
        payload.extend_from_slice(&q.x.to_le_bytes());
        payload.extend_from_slice(&q.y.to_le_bytes());
        payload.extend_from_slice(&q.z.to_le_bytes());
        payload.extend_from_slice(&q.w.to_le_bytes());
    }
    // Section 3: colors
    for c in cloud.color() {
        payload.extend_from_slice(&c.x.to_le_bytes());
        payload.extend_from_slice(&c.y.to_le_bytes());
        payload.extend_from_slice(&c.z.to_le_bytes());
    }
    // Section 4: opacities
    for a in cloud.opacity() {
        payload.extend_from_slice(&a.to_le_bytes());
    }
    // Section 5: SH (optional)
    if let Some(sh) = cloud.sh() {
        for splat_sh in sh {
            for coef in splat_sh {
                payload.extend_from_slice(&coef.to_le_bytes());
            }
        }
    }

    let digest = blake3::hash(&payload);
    let digest_truncated: [u8; 8] = digest.as_bytes()[..8].try_into().unwrap();

    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    let flags = if has_sh { FLAG_HAS_SH } else { 0 };
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&digest_truncated);
    out.extend_from_slice(&payload);
    out
}

/// Decode a [`SplatCloud`] from its on-disk byte representation.
pub fn decode(bytes: &[u8]) -> Result<SplatCloud, AssetError> {
    let header = decode_header(bytes)?;
    let payload = &bytes[HEADER_BYTES..];

    if payload.len() != header.payload_bytes as usize {
        return Err(AssetError::PayloadSizeMismatch {
            expected: header.payload_bytes,
            actual: payload.len() as u32,
        });
    }

    // Verify the truncated digest matches.
    let computed = blake3::hash(payload);
    if computed.as_bytes()[..8] != header.digest_truncated {
        return Err(AssetError::DigestMismatch);
    }

    let n = header.splat_count as usize;
    let has_sh = header.flags & FLAG_HAS_SH != 0;
    let mut cursor = 0usize;

    // Section 0: positions
    let mut positions = Vec::with_capacity(n);
    for _ in 0..n {
        let p = read_vec3(payload, &mut cursor)?;
        positions.push(p);
    }
    // Section 1: scales
    let mut scales = Vec::with_capacity(n);
    for _ in 0..n {
        let s = read_vec3(payload, &mut cursor)?;
        scales.push(s);
    }
    // Section 2: rotations
    let mut rotations = Vec::with_capacity(n);
    for _ in 0..n {
        let q = read_quat(payload, &mut cursor)?;
        rotations.push(q);
    }
    // Section 3: colors
    let mut colors = Vec::with_capacity(n);
    for _ in 0..n {
        let c = read_vec3(payload, &mut cursor)?;
        colors.push(c);
    }
    // Section 4: opacities
    let mut opacities = Vec::with_capacity(n);
    for _ in 0..n {
        let a = read_f32(payload, &mut cursor)?;
        opacities.push(a);
    }
    // Section 5: SH (optional)
    let sh = if has_sh {
        let mut sh_vec = Vec::with_capacity(n);
        for _ in 0..n {
            let mut splat_sh = [0.0f32; SH_COEFFS_PER_CHANNEL * 3];
            for slot in splat_sh.iter_mut() {
                *slot = read_f32(payload, &mut cursor)?;
            }
            sh_vec.push(splat_sh);
        }
        Some(sh_vec)
    } else {
        None
    };

    let cloud = SplatCloudBuilder::with_capacity(n)
        .positions(positions)
        .scales(scales)
        .rotations(rotations)
        .colors(colors)
        .opacities(opacities);
    let cloud = if let Some(sh) = sh {
        cloud.spherical_harmonics(sh)
    } else {
        cloud
    };
    cloud.build().map_err(AssetError::from)
}

/// Decode the per-cloud metadata without materialising the full
/// [`SplatCloud`]. Convenient for asset-pipeline tools.
pub fn decode_meta(bytes: &[u8]) -> Result<SplatCloudMeta, AssetError> {
    let header = decode_header(bytes)?;
    if bytes.len() < HEADER_BYTES + header.payload_bytes as usize {
        return Err(AssetError::PayloadSizeMismatch {
            expected: header.payload_bytes,
            actual: (bytes.len() - HEADER_BYTES) as u32,
        });
    }
    let payload = &bytes[HEADER_BYTES..HEADER_BYTES + header.payload_bytes as usize];
    let digest = blake3::hash(payload);
    Ok(SplatCloudMeta {
        splat_count: header.splat_count,
        has_sh: header.flags & FLAG_HAS_SH != 0,
        payload_digest: *digest.as_bytes(),
    })
}

#[derive(Clone, Copy, Debug)]
struct Header {
    flags: u16,
    splat_count: u32,
    payload_bytes: u32,
    digest_truncated: [u8; 8],
}

fn decode_header(bytes: &[u8]) -> Result<Header, AssetError> {
    if bytes.len() < HEADER_BYTES {
        return Err(AssetError::HeaderTruncated);
    }
    if &bytes[..4] != MAGIC.as_slice() {
        return Err(AssetError::WrongMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(AssetError::UnsupportedVersion(version));
    }
    let flags = u16::from_le_bytes([bytes[6], bytes[7]]);
    // Reject any flag bits beyond the documented set so old readers
    // refuse forward-incompatible files cleanly.
    if flags & !FLAG_HAS_SH != 0 {
        return Err(AssetError::UnknownFlagBit(flags));
    }
    let splat_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let payload_bytes = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let digest_truncated: [u8; 8] = bytes[16..24].try_into().unwrap();
    Ok(Header {
        flags,
        splat_count,
        payload_bytes,
        digest_truncated,
    })
}

fn read_f32(payload: &[u8], cursor: &mut usize) -> Result<f32, AssetError> {
    if *cursor + 4 > payload.len() {
        return Err(AssetError::PayloadTruncated);
    }
    let arr: [u8; 4] = payload[*cursor..*cursor + 4].try_into().unwrap();
    *cursor += 4;
    Ok(f32::from_le_bytes(arr))
}

fn read_vec3(payload: &[u8], cursor: &mut usize) -> Result<Vec3, AssetError> {
    let x = read_f32(payload, cursor)?;
    let y = read_f32(payload, cursor)?;
    let z = read_f32(payload, cursor)?;
    Ok(Vec3::new(x, y, z))
}

fn read_quat(payload: &[u8], cursor: &mut usize) -> Result<Quat, AssetError> {
    let x = read_f32(payload, cursor)?;
    let y = read_f32(payload, cursor)?;
    let z = read_f32(payload, cursor)?;
    let w = read_f32(payload, cursor)?;
    Ok(Quat::new(x, y, z, w))
}

/// ESPL asset decode error variants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetError {
    /// File is too small to hold the 24-byte header.
    HeaderTruncated,
    /// First 4 bytes don't match `"ESPL"`.
    WrongMagic,
    /// Header version is not [`VERSION`].
    UnsupportedVersion(u16),
    /// Header flag word has bits set the current version does not
    /// understand (forward-compat guard per ADR-078 §1).
    UnknownFlagBit(u16),
    /// `payload_bytes` in header doesn't match the body slice.
    PayloadSizeMismatch {
        /// Expected payload length from header.
        expected: u32,
        /// Actual payload length on disk.
        actual: u32,
    },
    /// Truncated BLAKE3 digest in header doesn't match the payload's
    /// freshly-computed digest.
    DigestMismatch,
    /// Payload ran out before all sections were decoded.
    PayloadTruncated,
    /// `SplatCloudBuilder::build()` rejected the decoded attributes
    /// (shouldn't happen on well-formed files since the decode walks
    /// `splat_count` for every section).
    CloudConstruction(crate::cloud::CloudError),
}

impl From<crate::cloud::CloudError> for AssetError {
    fn from(e: crate::cloud::CloudError) -> Self {
        AssetError::CloudConstruction(e)
    }
}

impl core::fmt::Display for AssetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AssetError::HeaderTruncated => write!(f, "ESPL header truncated (need ≥ 24 bytes)"),
            AssetError::WrongMagic => write!(f, "ESPL header has wrong magic (expected \"ESPL\")"),
            AssetError::UnsupportedVersion(v) => write!(f, "ESPL unsupported version {v}"),
            AssetError::UnknownFlagBit(flags) => {
                write!(f, "ESPL header has unknown flag bits: {flags:#x}")
            }
            AssetError::PayloadSizeMismatch { expected, actual } => write!(
                f,
                "ESPL payload size mismatch: header says {expected}, found {actual}"
            ),
            AssetError::DigestMismatch => write!(f, "ESPL payload digest mismatch"),
            AssetError::PayloadTruncated => write!(f, "ESPL payload ran out during decode"),
            AssetError::CloudConstruction(e) => write!(f, "ESPL cloud construction: {e}"),
        }
    }
}

impl std::error::Error for AssetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::SplatCloudBuilder;
    use engine_math::{Quat, Vec3};

    fn small_cloud(with_sh: bool) -> SplatCloud {
        let n = 3;
        let positions = vec![
            Vec3::new(0.0, 1.0, 2.0),
            Vec3::new(3.0, 4.0, 5.0),
            Vec3::new(6.0, 7.0, 8.0),
        ];
        let scales = vec![Vec3::new(0.1, 0.1, 0.1); n];
        let rotations = vec![Quat::IDENTITY; n];
        let colors = vec![
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let opacities = vec![0.5, 0.6, 0.7];
        let b = SplatCloudBuilder::with_capacity(n)
            .positions(positions)
            .scales(scales)
            .rotations(rotations)
            .colors(colors)
            .opacities(opacities);
        let b = if with_sh {
            b.spherical_harmonics(vec![[0.25; 27]; n])
        } else {
            b
        };
        b.build().expect("builds")
    }

    #[test]
    fn round_trip_without_sh() {
        let cloud = small_cloud(false);
        let bytes = encode(&cloud);
        let decoded = decode(&bytes).expect("decodes");
        assert_eq!(decoded.len(), cloud.len());
        for i in 0..cloud.len() {
            assert_eq!(decoded.position()[i], cloud.position()[i]);
            assert_eq!(decoded.opacity()[i], cloud.opacity()[i]);
        }
        assert!(decoded.sh().is_none());
    }

    #[test]
    fn round_trip_with_sh() {
        let cloud = small_cloud(true);
        let bytes = encode(&cloud);
        let decoded = decode(&bytes).expect("decodes");
        assert!(decoded.sh().is_some());
        assert_eq!(decoded.sh().unwrap()[0][13], 0.25);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = encode(&small_cloud(false));
        bytes[0] = b'X';
        assert_eq!(decode(&bytes).unwrap_err(), AssetError::WrongMagic);
    }

    #[test]
    fn rejects_corrupt_payload() {
        let mut bytes = encode(&small_cloud(false));
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert_eq!(decode(&bytes).unwrap_err(), AssetError::DigestMismatch);
    }

    #[test]
    fn rejects_unknown_flag_bit() {
        let mut bytes = encode(&small_cloud(false));
        // Set an undocumented flag bit (bit 1, the "compressed" reserved bit).
        bytes[6] |= 0b10;
        assert!(matches!(
            decode(&bytes).unwrap_err(),
            AssetError::UnknownFlagBit(_)
        ));
    }

    #[test]
    fn meta_decode_matches_full_decode() {
        let cloud = small_cloud(true);
        let bytes = encode(&cloud);
        let meta = decode_meta(&bytes).expect("meta decodes");
        assert_eq!(meta.splat_count, cloud.len() as u32);
        assert!(meta.has_sh);
        // Digest is 32 bytes (full BLAKE3); the header stores 8.
        assert_eq!(meta.payload_digest.len(), 32);
    }

    #[test]
    fn header_size_is_24_bytes() {
        let bytes = encode(&small_cloud(false));
        // Header is 24, payload begins at byte 24.
        assert!(bytes.len() > HEADER_BYTES);
        assert_eq!(&bytes[..4], b"ESPL");
    }
}
