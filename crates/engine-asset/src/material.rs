//! Material asset metadata (ADR-061 §2).
//!
//! A compiled material in a [`Pak`](crate::Pak) is a single blob with
//! a fixed-layout [`MaterialMeta`] header followed by the typed
//! payload: a [`TextureSlot`] table and a `f32` factors table.
//!
//! Per ADR-061 §2. Owned-discipline: hand-written little-endian
//! codec, no serde, reserved bytes are validated to be zero on decode
//! so the v1 hash remains stable as future v2 expansion reclaims
//! reserved bytes.

use crate::hash::ContentHash;
use crate::pak::PakError;

/// Magic for the material-blob header. ASCII `EMAT`, little-endian.
const MAT_MAGIC: [u8; 4] = *b"EMAT";

/// Bytes consumed by an encoded [`MaterialMeta`] — kept in lockstep
/// with [`MaterialMeta::encode`].
pub const MATERIAL_META_BYTES: usize = 4   // magic
    + 2 // version
    + 2 // flags
    + 4 // shader_id
    + 4 // payload_len
    + 1 // texture_count
    + 1 // factor_count
    + 6; // reserved (zeroed in v1)

/// Bytes per [`TextureSlot`] in the encoded material payload.
pub const TEXTURE_SLOT_BYTES: usize = 1   // semantic
    + 1 // sampler_kind
    + 6 // reserved (zeroed in v1)
    + 32; // content_hash

/// Cap on the number of [`TextureSlot`] records per material. Higher
/// counts trigger a decode rejection.
pub const MAX_TEXTURE_SLOTS: u8 = 16;

/// Cap on the number of `f32` factors per material. Higher counts
/// trigger a decode rejection.
pub const MAX_FACTORS: u8 = 32;

/// What a material's bound texture slot is *for* — drives sampler
/// selection and shader-side semantics. Discriminants intentionally
/// match [`super::texture::ChannelRole`] so a texture's intent and a
/// material's expectation can be compared without translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum TextureSemantic {
    /// Albedo / diffuse / emissive. sRGB-encoded.
    Albedo = 0,
    /// Tangent-space normal. Linear; Z reconstructed in shader.
    Normal = 1,
    /// Packed roughness+metallic+AO. Linear.
    RoughMetAo = 2,
    /// HDR cubemap (IBL specular probes).
    Hdr = 3,
    /// UI / pre-multiplied-alpha sprite. sRGB.
    Ui = 4,
}

impl TextureSemantic {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(TextureSemantic::Albedo),
            1 => Some(TextureSemantic::Normal),
            2 => Some(TextureSemantic::RoughMetAo),
            3 => Some(TextureSemantic::Hdr),
            4 => Some(TextureSemantic::Ui),
            _ => None,
        }
    }
}

/// Sampler filter / address-mode preset attached to a [`TextureSlot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum SamplerKind {
    /// Bilinear filtering, repeat address mode. Default for color
    /// textures.
    Linear = 0,
    /// 16× anisotropic filtering, repeat address mode. For
    /// ground/wall textures.
    Anisotropic = 1,
    /// Nearest-neighbour filtering, clamp address mode. For pixel art
    /// / data textures.
    Point = 2,
}

impl SamplerKind {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(SamplerKind::Linear),
            1 => Some(SamplerKind::Anisotropic),
            2 => Some(SamplerKind::Point),
            _ => None,
        }
    }
}

/// One texture binding in a material. Resolves at load time to a
/// [`super::texture::TextureMeta`] in the same pak via
/// [`content_hash`](Self::content_hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureSlot {
    /// What the bound texture is for.
    pub semantic: TextureSemantic,
    /// Sampler preset to use when sampling the texture.
    pub sampler_kind: SamplerKind,
    /// Content hash of the bound texture asset (in the same pak).
    pub content_hash: ContentHash,
}

/// On-disk material metadata. Carried as a fixed-size 24-byte header
/// at the start of the pak blob; the remainder is the texture-slot
/// table followed by the scalar factors.
///
/// The schema is fixed (no growth-room beyond the v1 `flags`); future
/// expansion bumps the magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialMeta {
    /// Truncated 32-bit prefix of the target Slang artefact's digest
    /// (ADR-037 §Artefact format). The runtime looks the full digest
    /// up in the pak; a mismatch is a hard load error.
    pub shader_id: u32,
    /// Number of [`TextureSlot`] records in the payload.
    pub texture_count: u8,
    /// Number of `f32` scalar factors in the payload.
    pub factor_count: u8,
}

impl MaterialMeta {
    /// Encode the header into a fixed-size byte buffer of length
    /// [`MATERIAL_META_BYTES`]. `payload_len` is the total byte
    /// length of the texture-slot table + scalar factors.
    pub fn encode(self, payload_len: u32) -> [u8; MATERIAL_META_BYTES] {
        let mut out = [0u8; MATERIAL_META_BYTES];
        let mut i = 0;
        out[i..i + 4].copy_from_slice(&MAT_MAGIC);
        i += 4;
        out[i..i + 2].copy_from_slice(&1u16.to_le_bytes()); // version
        i += 2;
        out[i..i + 2].copy_from_slice(&0u16.to_le_bytes()); // flags
        i += 2;
        out[i..i + 4].copy_from_slice(&self.shader_id.to_le_bytes());
        i += 4;
        out[i..i + 4].copy_from_slice(&payload_len.to_le_bytes());
        i += 4;
        out[i] = self.texture_count;
        i += 1;
        out[i] = self.factor_count;
        // remaining 6 bytes already zero per array init
        out
    }

    /// Decode the header. Returns `(meta, payload)`. Validates that
    /// reserved header bytes are zero so future v2 expansion can
    /// reclaim them without changing v1 hashes.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), PakError> {
        if bytes.len() < MATERIAL_META_BYTES {
            return Err(PakError::Truncated);
        }
        if bytes[0..4] != MAT_MAGIC {
            return Err(PakError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != 1 {
            return Err(PakError::UnsupportedVersion(version.into()));
        }
        let _flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let shader_id = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let payload_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        let texture_count = bytes[16];
        let factor_count = bytes[17];

        if bytes[18..MATERIAL_META_BYTES].iter().any(|&b| b != 0) {
            return Err(PakError::Truncated);
        }

        if texture_count > MAX_TEXTURE_SLOTS || factor_count > MAX_FACTORS {
            return Err(PakError::OutOfBounds);
        }

        let payload = &bytes[MATERIAL_META_BYTES..];
        if payload.len() != payload_len {
            return Err(PakError::OutOfBounds);
        }
        Ok((
            Self {
                shader_id,
                texture_count,
                factor_count,
            },
            payload,
        ))
    }

    /// Expected payload byte length: `texture_count * TEXTURE_SLOT_BYTES
    /// + factor_count * 4`.
    pub fn expected_payload_len(self) -> u64 {
        u64::from(self.texture_count) * TEXTURE_SLOT_BYTES as u64 + u64::from(self.factor_count) * 4
    }
}

/// Encode a [`TextureSlot`] into [`TEXTURE_SLOT_BYTES`] bytes.
pub fn encode_texture_slot(slot: TextureSlot) -> [u8; TEXTURE_SLOT_BYTES] {
    let mut out = [0u8; TEXTURE_SLOT_BYTES];
    out[0] = slot.semantic as u8;
    out[1] = slot.sampler_kind as u8;
    // bytes 2..8 are reserved, zeroed by array init.
    out[8..40].copy_from_slice(slot.content_hash.as_bytes());
    out
}

/// Decode a [`TextureSlot`] from [`TEXTURE_SLOT_BYTES`] bytes.
pub fn decode_texture_slot(bytes: &[u8]) -> Result<TextureSlot, PakError> {
    if bytes.len() < TEXTURE_SLOT_BYTES {
        return Err(PakError::Truncated);
    }
    let semantic = TextureSemantic::from_u8(bytes[0]).ok_or(PakError::Truncated)?;
    let sampler_kind = SamplerKind::from_u8(bytes[1]).ok_or(PakError::Truncated)?;
    if bytes[2..8].iter().any(|&b| b != 0) {
        return Err(PakError::Truncated);
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes[8..40]);
    Ok(TextureSlot {
        semantic,
        sampler_kind,
        content_hash: ContentHash::from_bytes(hash),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MaterialMeta {
        MaterialMeta {
            shader_id: 0xdead_beef,
            texture_count: 2,
            factor_count: 4,
        }
    }

    fn sample_payload(meta: MaterialMeta) -> Vec<u8> {
        let hash = ContentHash::of(b"sample-texture");
        let slot = TextureSlot {
            semantic: TextureSemantic::Albedo,
            sampler_kind: SamplerKind::Linear,
            content_hash: hash,
        };
        let mut payload = Vec::new();
        for _ in 0..meta.texture_count {
            payload.extend_from_slice(&encode_texture_slot(slot));
        }
        for i in 0..meta.factor_count {
            payload.extend_from_slice(&(i as f32).to_le_bytes());
        }
        payload
    }

    #[test]
    fn encode_decode_round_trips() {
        let meta = sample();
        let payload = sample_payload(meta);
        let mut blob = Vec::new();
        blob.extend_from_slice(&meta.encode(payload.len() as u32));
        blob.extend_from_slice(&payload);

        let (decoded, payload_out) = MaterialMeta::decode(&blob).expect("valid blob");
        assert_eq!(decoded, meta);
        assert_eq!(payload_out.len(), payload.len());
    }

    #[test]
    fn expected_payload_len_matches_encoded() {
        let meta = sample();
        let payload = sample_payload(meta);
        assert_eq!(meta.expected_payload_len(), payload.len() as u64);
    }

    #[test]
    fn header_length_is_stable() {
        let meta = sample();
        let encoded = meta.encode(0);
        assert_eq!(encoded.len(), MATERIAL_META_BYTES);
        assert_eq!(&encoded[0..4], b"EMAT");
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut blob = sample().encode(0).to_vec();
        blob[0] = b'X';
        assert_eq!(MaterialMeta::decode(&blob), Err(PakError::BadMagic));
    }

    #[test]
    fn decode_rejects_nonzero_reserved() {
        let mut blob = sample().encode(0).to_vec();
        blob[18] = 1; // first reserved byte
        assert_eq!(MaterialMeta::decode(&blob), Err(PakError::Truncated));
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut blob = sample().encode(0).to_vec();
        blob[4] = 9;
        assert_eq!(
            MaterialMeta::decode(&blob),
            Err(PakError::UnsupportedVersion(9))
        );
    }

    #[test]
    fn decode_rejects_too_many_textures() {
        let mut meta = sample();
        meta.texture_count = MAX_TEXTURE_SLOTS + 1;
        let blob = meta.encode(0);
        assert_eq!(MaterialMeta::decode(&blob), Err(PakError::OutOfBounds));
    }

    #[test]
    fn decode_rejects_too_many_factors() {
        let mut meta = sample();
        meta.factor_count = MAX_FACTORS + 1;
        let blob = meta.encode(0);
        assert_eq!(MaterialMeta::decode(&blob), Err(PakError::OutOfBounds));
    }

    #[test]
    fn texture_slot_round_trips() {
        let hash = ContentHash::of(b"test");
        let slot = TextureSlot {
            semantic: TextureSemantic::Normal,
            sampler_kind: SamplerKind::Anisotropic,
            content_hash: hash,
        };
        let bytes = encode_texture_slot(slot);
        assert_eq!(decode_texture_slot(&bytes).unwrap(), slot);
    }

    #[test]
    fn texture_slot_rejects_nonzero_reserved() {
        let hash = ContentHash::of(b"x");
        let slot = TextureSlot {
            semantic: TextureSemantic::Albedo,
            sampler_kind: SamplerKind::Linear,
            content_hash: hash,
        };
        let mut bytes = encode_texture_slot(slot).to_vec();
        bytes[2] = 1; // reserved
        assert_eq!(decode_texture_slot(&bytes), Err(PakError::Truncated));
    }

    #[test]
    fn texture_slot_rejects_unknown_semantic() {
        let mut bytes = vec![0u8; TEXTURE_SLOT_BYTES];
        bytes[0] = 0xff;
        assert_eq!(decode_texture_slot(&bytes), Err(PakError::Truncated));
    }
}
