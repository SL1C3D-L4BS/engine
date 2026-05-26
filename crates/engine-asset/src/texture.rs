//! Texture asset metadata (ADR-045 §4).
//!
//! A compiled texture in a [`Pak`](crate::Pak) is a single blob with a
//! fixed-layout [`TextureMeta`] header followed by the compressed BC bytes.
//! [`TextureMeta::encode`] / [`TextureMeta::decode`] are deterministic and
//! reversible; the pak format (deterministic / content-addressed) is
//! unaffected — the header is just leading bytes in the blob.
//!
//! The format enum here is a deliberate duplicate of `engine_gpu::TextureFormat`
//! (the BC subset): engine-asset and engine-gpu are both Level-1 crates and
//! must not depend on each other. The mapping is one-way at load time
//! (`engine_gpu::TextureFormat` consumes [`TexFormat`]), keeping the asset
//! layer free of any GPU dependency.

use crate::pak::PakError;

/// On-disk texture pixel format.
///
/// Mirrors the BC subset of `engine_gpu::TextureFormat` per ADR-045 §2; the
/// uncompressed `Rgba8Unorm` variant is for the bindless slot-0 fallback
/// magenta only (ADR-045 §2 table row "Special").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u16)]
pub enum TexFormat {
    /// 1×1 magenta RGBA8 fallback. Block-size 1 (uncompressed).
    Rgba8Unorm = 0,
    /// BC4 — single-channel (roughness, metallic, AO). 8 bytes / 4×4 block.
    Bc4RUnorm = 1,
    /// BC5 — two-channel (tangent-space normals). 16 bytes / 4×4 block.
    Bc5RgUnorm = 2,
    /// BC6H — HDR cubemaps / IBL specular probes. 16 bytes / 4×4 block.
    Bc6hRgbUfloat = 3,
    /// BC7 sRGB — albedo / diffuse / UI. 16 bytes / 4×4 block.
    Bc7RgbaUnormSrgb = 4,
    /// BC7 linear — packed roughness+metallic+AO. 16 bytes / 4×4 block.
    Bc7RgbaUnorm = 5,
}

impl TexFormat {
    /// Bytes per 4×4 block. `1` for the uncompressed fallback.
    pub fn block_bytes(self) -> u32 {
        match self {
            TexFormat::Rgba8Unorm => 4, // per-texel
            TexFormat::Bc4RUnorm => 8,
            TexFormat::Bc5RgUnorm
            | TexFormat::Bc6hRgbUfloat
            | TexFormat::Bc7RgbaUnormSrgb
            | TexFormat::Bc7RgbaUnorm => 16,
        }
    }

    /// `true` for the BC{4,5,6,7} family (everything except the uncompressed
    /// fallback).
    pub fn is_bc(self) -> bool {
        !matches!(self, TexFormat::Rgba8Unorm)
    }

    fn from_u16(v: u16) -> Option<Self> {
        Some(match v {
            0 => TexFormat::Rgba8Unorm,
            1 => TexFormat::Bc4RUnorm,
            2 => TexFormat::Bc5RgUnorm,
            3 => TexFormat::Bc6hRgbUfloat,
            4 => TexFormat::Bc7RgbaUnormSrgb,
            5 => TexFormat::Bc7RgbaUnorm,
            _ => return None,
        })
    }
}

/// What a texture *is* (intent). Independent of [`TexFormat`] so debug
/// builds can ship uncompressed albedo and still hit the right sampler /
/// shader path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum ChannelRole {
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

impl ChannelRole {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => ChannelRole::Albedo,
            1 => ChannelRole::Normal,
            2 => ChannelRole::RoughMetAo,
            3 => ChannelRole::Hdr,
            4 => ChannelRole::Ui,
            _ => return None,
        })
    }
}

/// Width × height × layer-count for a 2D / 2D-array / cubemap texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TexExtent {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// 1 for 2D; 6 for cubemap; N for 2D array.
    pub layers: u32,
}

/// Magic for the texture-blob header. ASCII `ETEX`, little-endian.
const TEX_MAGIC: [u8; 4] = *b"ETEX";

/// Bytes consumed by an encoded [`TextureMeta`] — kept in lockstep with
/// [`TextureMeta::encode`].
pub const TEXTURE_META_BYTES: usize = 4   // magic
    + 2 // format
    + 1 // channel_role
    + 1 // mip_count
    + 4 + 4 + 4 // extent
    + 4; // payload length (informational; the pak entry already knows its size)

/// On-disk texture metadata. Carried as a fixed-size header at the start of
/// the pak blob; the remainder is the compressed BC payload.
///
/// Per ADR-045 §4. The schema is fixed (no growth-room field); future
/// expansion bumps the magic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureMeta {
    /// On-disk codec.
    pub format: TexFormat,
    /// Texture extent.
    pub extent: TexExtent,
    /// Mip-level count. Always ≥ 1; full chains down to 1×1 per ADR-045 §3.
    pub mip_count: u8,
    /// What this texture is for (sampler / shader-path selection).
    pub channel_role: ChannelRole,
}

impl TextureMeta {
    /// Encode the header into a fixed-size byte buffer of length
    /// [`TEXTURE_META_BYTES`]. `payload_len` is the byte length of the
    /// compressed BC bytes that will follow; it is written into the header
    /// so a decoder can validate the buffer length matches expectations.
    pub fn encode(self, payload_len: u32) -> [u8; TEXTURE_META_BYTES] {
        let mut out = [0u8; TEXTURE_META_BYTES];
        let mut i = 0;
        out[i..i + 4].copy_from_slice(&TEX_MAGIC);
        i += 4;
        out[i..i + 2].copy_from_slice(&(self.format as u16).to_le_bytes());
        i += 2;
        out[i] = self.channel_role as u8;
        i += 1;
        out[i] = self.mip_count;
        i += 1;
        out[i..i + 4].copy_from_slice(&self.extent.width.to_le_bytes());
        i += 4;
        out[i..i + 4].copy_from_slice(&self.extent.height.to_le_bytes());
        i += 4;
        out[i..i + 4].copy_from_slice(&self.extent.layers.to_le_bytes());
        i += 4;
        out[i..i + 4].copy_from_slice(&payload_len.to_le_bytes());
        out
    }

    /// Decode the header from a blob and return `(meta, payload)`. The
    /// payload slice is the remainder of `bytes` after the header — its
    /// length is validated against the header's recorded payload length.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), PakError> {
        if bytes.len() < TEXTURE_META_BYTES {
            return Err(PakError::Truncated);
        }
        if bytes[0..4] != TEX_MAGIC {
            return Err(PakError::BadMagic);
        }
        let format = u16::from_le_bytes([bytes[4], bytes[5]]);
        let format = TexFormat::from_u16(format).ok_or(PakError::Truncated)?;
        let channel_role = ChannelRole::from_u8(bytes[6]).ok_or(PakError::Truncated)?;
        let mip_count = bytes[7];
        if mip_count == 0 {
            return Err(PakError::Truncated);
        }
        let width = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let height = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let layers = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let payload_len = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;
        let payload = &bytes[TEXTURE_META_BYTES..];
        if payload.len() != payload_len {
            return Err(PakError::OutOfBounds);
        }
        Ok((
            Self {
                format,
                extent: TexExtent {
                    width,
                    height,
                    layers,
                },
                mip_count,
                channel_role,
            },
            payload,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TextureMeta {
        TextureMeta {
            format: TexFormat::Bc7RgbaUnormSrgb,
            extent: TexExtent {
                width: 256,
                height: 128,
                layers: 1,
            },
            mip_count: 9,
            channel_role: ChannelRole::Albedo,
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let meta = sample();
        let payload = vec![0xABu8; 4096];
        let mut blob = Vec::with_capacity(TEXTURE_META_BYTES + payload.len());
        blob.extend_from_slice(&meta.encode(payload.len() as u32));
        blob.extend_from_slice(&payload);

        let (decoded, payload_out) = TextureMeta::decode(&blob).expect("valid blob");
        assert_eq!(decoded, meta);
        assert_eq!(payload_out, &payload[..]);
    }

    #[test]
    fn header_length_is_stable() {
        let meta = sample();
        let encoded = meta.encode(0);
        assert_eq!(encoded.len(), TEXTURE_META_BYTES);
        assert_eq!(&encoded[0..4], b"ETEX");
    }

    #[test]
    fn decode_rejects_short_blob() {
        let too_short = vec![0u8; TEXTURE_META_BYTES - 1];
        assert_eq!(TextureMeta::decode(&too_short), Err(PakError::Truncated));
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut blob = sample().encode(0).to_vec();
        blob[0] = b'X';
        assert_eq!(TextureMeta::decode(&blob), Err(PakError::BadMagic));
    }

    #[test]
    fn decode_rejects_payload_length_mismatch() {
        let payload = vec![0u8; 32];
        let mut blob = sample().encode(64).to_vec(); // header claims 64 bytes
        blob.extend_from_slice(&payload); // but only 32 follow
        assert_eq!(TextureMeta::decode(&blob), Err(PakError::OutOfBounds));
    }

    #[test]
    fn decode_rejects_zero_mip_count() {
        let mut bad = sample();
        bad.mip_count = 0;
        let blob = bad.encode(0);
        assert_eq!(TextureMeta::decode(&blob), Err(PakError::Truncated));
    }

    #[test]
    fn decode_rejects_unknown_format() {
        let mut blob = sample().encode(0).to_vec();
        // Set the format word to an unknown discriminant.
        blob[4] = 0xff;
        blob[5] = 0xff;
        assert_eq!(TextureMeta::decode(&blob), Err(PakError::Truncated));
    }

    #[test]
    fn block_bytes_table() {
        assert_eq!(TexFormat::Rgba8Unorm.block_bytes(), 4);
        assert_eq!(TexFormat::Bc4RUnorm.block_bytes(), 8);
        assert_eq!(TexFormat::Bc5RgUnorm.block_bytes(), 16);
        assert_eq!(TexFormat::Bc7RgbaUnormSrgb.block_bytes(), 16);
        assert!(TexFormat::Bc4RUnorm.is_bc());
        assert!(!TexFormat::Rgba8Unorm.is_bc());
    }
}
