//! Mesh asset metadata (ADR-061 §1).
//!
//! A compiled mesh in a [`Pak`](crate::Pak) is a single blob with a
//! fixed-layout [`MeshMeta`] header followed by the typed payload:
//! vertex buffer, index buffer, sub-mesh table, AABB. The header has
//! the same owned-discipline as [`super::texture::TextureMeta`] — no
//! serde, hand-written little-endian codec, all fields deterministic.
//!
//! Per ADR-061 §1. The schema is fixed (no growth-room field beyond
//! the v1 `flags`); future expansion bumps the magic.

use crate::pak::PakError;

/// Magic for the mesh-blob header. ASCII `EMSH`, little-endian.
const MESH_MAGIC: [u8; 4] = *b"EMSH";

/// Bytes consumed by an encoded [`MeshMeta`] — kept in lockstep with
/// [`MeshMeta::encode`].
pub const MESH_META_BYTES: usize = 4   // magic
    + 2 // version
    + 2 // flags
    + 4 // vertex_count
    + 4 // index_count
    + 1 // vertex_stride
    + 1 // index_format
    + 1 // semantic_mask
    + 1 // sub_mesh_count
    + 4; // payload_len (informational; the pak entry already knows its size)

/// Bytes per [`SubMesh`] record in the encoded mesh payload.
pub const SUB_MESH_BYTES: usize = 12;

/// Bytes per AABB record (six little-endian f32 values).
pub const AABB_BYTES: usize = 24;

/// Cap on the number of [`SubMesh`] records per mesh. Higher counts
/// trigger a decode rejection.
pub const MAX_SUB_MESHES: u8 = 16;

/// Index buffer format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum IndexFormat {
    /// 16-bit unsigned indices. 2 bytes each.
    U16 = 0,
    /// 32-bit unsigned indices. 4 bytes each.
    U32 = 1,
}

impl IndexFormat {
    /// Bytes per index value.
    pub fn bytes(self) -> usize {
        match self {
            IndexFormat::U16 => 2,
            IndexFormat::U32 => 4,
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(IndexFormat::U16),
            1 => Some(IndexFormat::U32),
            _ => None,
        }
    }
}

/// Vertex attribute semantics, ordered by their bit position in the
/// [`MeshMeta::semantic_mask`].
///
/// A mesh's vertex payload is laid out per-vertex in bit order: for
/// each set bit (LSB first), the corresponding semantic's bytes appear
/// before the next semantic's. A semantic absent from the mask is
/// absent from the per-vertex stride.
///
/// The order is **fixed** for the v1 format — re-numbering existing
/// bits is a breaking change to the asset pak. New semantics may
/// appear in unused bit slots in a v2 format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum VertexSemantic {
    /// Object-space position; 3 × f32 LE (12 B).
    Position = 0,
    /// Object-space normal; 3 × f32 LE (12 B).
    Normal = 1,
    /// Object-space tangent; 4 × f32 LE (16 B), `w` carries the
    /// bitangent sign.
    Tangent = 2,
    /// Primary texture coordinates; 2 × f32 LE (8 B).
    Uv0 = 3,
    /// Secondary texture coordinates; 2 × f32 LE (8 B).
    Uv1 = 4,
    /// Primary vertex color; 4 × u8 (4 B). sRGB.
    Color0 = 5,
    /// Skinning weights; 4 × u8 normalized (4 B).
    BoneWeights = 6,
    /// Skinning indices; 4 × u8 (4 B).
    BoneIndices = 7,
}

impl VertexSemantic {
    /// Byte size of this semantic's per-vertex value.
    pub fn bytes(self) -> usize {
        match self {
            VertexSemantic::Position | VertexSemantic::Normal => 12,
            VertexSemantic::Tangent => 16,
            VertexSemantic::Uv0 | VertexSemantic::Uv1 => 8,
            VertexSemantic::Color0 | VertexSemantic::BoneWeights | VertexSemantic::BoneIndices => 4,
        }
    }

    /// All semantic variants in bit-position order.
    pub const ALL: [Self; 8] = [
        Self::Position,
        Self::Normal,
        Self::Tangent,
        Self::Uv0,
        Self::Uv1,
        Self::Color0,
        Self::BoneWeights,
        Self::BoneIndices,
    ];
}

/// Bitset of [`VertexSemantic`]s present in a mesh's per-vertex
/// payload. All 256 u8 values are valid bitsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct SemanticMask(u8);

impl SemanticMask {
    /// Empty mask (no semantics).
    pub const EMPTY: Self = Self(0);

    /// A mask containing exactly the given semantic.
    pub const fn from_semantic(s: VertexSemantic) -> Self {
        Self(1u8 << (s as u8))
    }

    /// Add a semantic to the mask.
    pub const fn with(self, s: VertexSemantic) -> Self {
        Self(self.0 | (1u8 << (s as u8)))
    }

    /// `true` if `s` is in the mask.
    pub const fn contains(self, s: VertexSemantic) -> bool {
        self.0 & (1u8 << (s as u8)) != 0
    }

    /// Raw bitset value.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Construct from raw bits.
    pub const fn from_bits(b: u8) -> Self {
        Self(b)
    }

    /// Sum of byte sizes of all semantics in the mask.
    pub fn vertex_stride(self) -> usize {
        let mut total = 0;
        for s in VertexSemantic::ALL {
            if self.contains(s) {
                total += s.bytes();
            }
        }
        total
    }

    /// Iterator over the semantics in this mask, in bit-position
    /// order.
    pub fn iter(self) -> impl Iterator<Item = VertexSemantic> {
        VertexSemantic::ALL
            .into_iter()
            .filter(move |s| self.contains(*s))
    }
}

/// A contiguous range of indices that share a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SubMesh {
    /// Offset into the index buffer where this sub-mesh starts.
    pub first_index: u32,
    /// Number of indices in this sub-mesh.
    pub index_count: u32,
    /// Index into the pak's [`MaterialMeta`](super::material::MaterialMeta) records.
    pub material_index: u16,
    /// Reserved; must be zero in v1.
    pub flags: u16,
}

/// On-disk mesh metadata. Carried as a fixed-size 24-byte header at
/// the start of the pak blob; the remainder is the vertex buffer
/// followed by the index buffer, sub-mesh table, and AABB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshMeta {
    /// Number of vertices in the buffer.
    pub vertex_count: u32,
    /// Number of indices in the buffer.
    pub index_count: u32,
    /// Bytes per vertex; equal to `semantic_mask.vertex_stride()`.
    pub vertex_stride: u8,
    /// Index buffer encoding.
    pub index_format: IndexFormat,
    /// Bitset of vertex attribute semantics present.
    pub semantic_mask: SemanticMask,
    /// Number of [`SubMesh`] records in the payload; capped at
    /// [`MAX_SUB_MESHES`].
    pub sub_mesh_count: u8,
}

impl MeshMeta {
    /// Encode the header into a fixed-size byte buffer of length
    /// [`MESH_META_BYTES`]. `payload_len` is the total byte length of
    /// the vertex buffer + index buffer + sub-mesh table + AABB.
    pub fn encode(self, payload_len: u32) -> [u8; MESH_META_BYTES] {
        let mut out = [0u8; MESH_META_BYTES];
        let mut i = 0;
        out[i..i + 4].copy_from_slice(&MESH_MAGIC);
        i += 4;
        out[i..i + 2].copy_from_slice(&1u16.to_le_bytes()); // version
        i += 2;
        out[i..i + 2].copy_from_slice(&0u16.to_le_bytes()); // flags
        i += 2;
        out[i..i + 4].copy_from_slice(&self.vertex_count.to_le_bytes());
        i += 4;
        out[i..i + 4].copy_from_slice(&self.index_count.to_le_bytes());
        i += 4;
        out[i] = self.vertex_stride;
        i += 1;
        out[i] = self.index_format as u8;
        i += 1;
        out[i] = self.semantic_mask.bits();
        i += 1;
        out[i] = self.sub_mesh_count;
        i += 1;
        out[i..i + 4].copy_from_slice(&payload_len.to_le_bytes());
        out
    }

    /// Decode the header from a blob and return `(meta, payload)`.
    /// The payload slice is the remainder of `bytes` after the
    /// header — its length is validated against the header's recorded
    /// payload length, and the recorded vertex stride is validated
    /// against the semantic mask.
    pub fn decode(bytes: &[u8]) -> Result<(Self, &[u8]), PakError> {
        if bytes.len() < MESH_META_BYTES {
            return Err(PakError::Truncated);
        }
        if bytes[0..4] != MESH_MAGIC {
            return Err(PakError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != 1 {
            return Err(PakError::UnsupportedVersion(version.into()));
        }
        let _flags = u16::from_le_bytes([bytes[6], bytes[7]]);
        let vertex_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let index_count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let vertex_stride = bytes[16];
        let index_format = IndexFormat::from_u8(bytes[17]).ok_or(PakError::Truncated)?;
        let semantic_mask = SemanticMask::from_bits(bytes[18]);
        let sub_mesh_count = bytes[19];
        let payload_len = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;

        if sub_mesh_count > MAX_SUB_MESHES {
            return Err(PakError::OutOfBounds);
        }
        let expected_stride = semantic_mask.vertex_stride();
        if expected_stride > u8::MAX as usize || expected_stride as u8 != vertex_stride {
            return Err(PakError::Truncated);
        }

        let payload = &bytes[MESH_META_BYTES..];
        if payload.len() != payload_len {
            return Err(PakError::OutOfBounds);
        }
        Ok((
            Self {
                vertex_count,
                index_count,
                vertex_stride,
                index_format,
                semantic_mask,
                sub_mesh_count,
            },
            payload,
        ))
    }

    /// Expected byte length of the payload for the given counts.
    pub fn expected_payload_len(self) -> u64 {
        let vertex_bytes = u64::from(self.vertex_count) * u64::from(self.vertex_stride);
        let index_bytes = u64::from(self.index_count) * self.index_format.bytes() as u64;
        let sub_mesh_bytes = u64::from(self.sub_mesh_count) * SUB_MESH_BYTES as u64;
        vertex_bytes + index_bytes + sub_mesh_bytes + AABB_BYTES as u64
    }
}

/// Encode a [`SubMesh`] into 12 bytes.
pub fn encode_sub_mesh(sm: SubMesh) -> [u8; SUB_MESH_BYTES] {
    let mut out = [0u8; SUB_MESH_BYTES];
    out[0..4].copy_from_slice(&sm.first_index.to_le_bytes());
    out[4..8].copy_from_slice(&sm.index_count.to_le_bytes());
    out[8..10].copy_from_slice(&sm.material_index.to_le_bytes());
    out[10..12].copy_from_slice(&sm.flags.to_le_bytes());
    out
}

/// Decode a [`SubMesh`] from a 12-byte slice.
pub fn decode_sub_mesh(bytes: &[u8]) -> Result<SubMesh, PakError> {
    if bytes.len() < SUB_MESH_BYTES {
        return Err(PakError::Truncated);
    }
    Ok(SubMesh {
        first_index: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        index_count: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        material_index: u16::from_le_bytes([bytes[8], bytes[9]]),
        flags: u16::from_le_bytes([bytes[10], bytes[11]]),
    })
}

/// Encode an axis-aligned bounding box `(min.xyz, max.xyz)` into
/// [`AABB_BYTES`] little-endian f32 bytes.
pub fn encode_aabb(min: [f32; 3], max: [f32; 3]) -> [u8; AABB_BYTES] {
    let mut out = [0u8; AABB_BYTES];
    let vals = [min[0], min[1], min[2], max[0], max[1], max[2]];
    for (i, v) in vals.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode an axis-aligned bounding box from [`AABB_BYTES`]
/// little-endian f32 bytes. Returns `(min, max)`.
pub fn decode_aabb(bytes: &[u8]) -> Result<([f32; 3], [f32; 3]), PakError> {
    if bytes.len() < AABB_BYTES {
        return Err(PakError::Truncated);
    }
    let mut vals = [0.0f32; 6];
    for (i, v) in vals.iter_mut().enumerate() {
        *v = f32::from_le_bytes([
            bytes[i * 4],
            bytes[i * 4 + 1],
            bytes[i * 4 + 2],
            bytes[i * 4 + 3],
        ]);
    }
    Ok(([vals[0], vals[1], vals[2]], [vals[3], vals[4], vals[5]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MeshMeta {
        let mask = SemanticMask::EMPTY
            .with(VertexSemantic::Position)
            .with(VertexSemantic::Normal)
            .with(VertexSemantic::Uv0);
        MeshMeta {
            vertex_count: 24,
            index_count: 36,
            vertex_stride: mask.vertex_stride() as u8,
            index_format: IndexFormat::U16,
            semantic_mask: mask,
            sub_mesh_count: 1,
        }
    }

    fn sample_payload(meta: MeshMeta) -> Vec<u8> {
        let stride = meta.vertex_stride as usize;
        let mut payload = vec![0u8; (meta.vertex_count as usize) * stride];
        payload.extend(std::iter::repeat_n(
            0u8,
            (meta.index_count as usize) * meta.index_format.bytes(),
        ));
        for i in 0..meta.sub_mesh_count {
            payload.extend_from_slice(&encode_sub_mesh(SubMesh {
                first_index: 0,
                index_count: meta.index_count,
                material_index: i.into(),
                flags: 0,
            }));
        }
        payload.extend_from_slice(&encode_aabb([0.0; 3], [1.0; 3]));
        payload
    }

    #[test]
    fn encode_decode_round_trips() {
        let meta = sample();
        let payload = sample_payload(meta);
        let mut blob = Vec::with_capacity(MESH_META_BYTES + payload.len());
        blob.extend_from_slice(&meta.encode(payload.len() as u32));
        blob.extend_from_slice(&payload);

        let (decoded, payload_out) = MeshMeta::decode(&blob).expect("valid blob");
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
        assert_eq!(encoded.len(), MESH_META_BYTES);
        assert_eq!(&encoded[0..4], b"EMSH");
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut blob = sample().encode(0).to_vec();
        blob[0] = b'X';
        assert_eq!(MeshMeta::decode(&blob), Err(PakError::BadMagic));
    }

    #[test]
    fn decode_rejects_short_blob() {
        let too_short = vec![0u8; MESH_META_BYTES - 1];
        assert_eq!(MeshMeta::decode(&too_short), Err(PakError::Truncated));
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut blob = sample().encode(0).to_vec();
        blob[4] = 2;
        assert_eq!(
            MeshMeta::decode(&blob),
            Err(PakError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn decode_rejects_stride_mismatch() {
        let mut meta = sample();
        meta.vertex_stride = 4; // wrong; mask wants 32
        let blob = meta.encode(0);
        assert_eq!(MeshMeta::decode(&blob), Err(PakError::Truncated));
    }

    #[test]
    fn decode_rejects_too_many_sub_meshes() {
        let mut meta = sample();
        meta.sub_mesh_count = MAX_SUB_MESHES + 1;
        let blob = meta.encode(0);
        assert_eq!(MeshMeta::decode(&blob), Err(PakError::OutOfBounds));
    }

    #[test]
    fn decode_rejects_unknown_index_format() {
        let mut blob = sample().encode(0).to_vec();
        blob[17] = 0xff;
        assert_eq!(MeshMeta::decode(&blob), Err(PakError::Truncated));
    }

    #[test]
    fn semantic_mask_iter_is_bit_ordered() {
        let mask = SemanticMask::EMPTY
            .with(VertexSemantic::Uv0)
            .with(VertexSemantic::Position)
            .with(VertexSemantic::Color0);
        let collected: Vec<_> = mask.iter().collect();
        assert_eq!(
            collected,
            vec![
                VertexSemantic::Position,
                VertexSemantic::Uv0,
                VertexSemantic::Color0,
            ]
        );
    }

    #[test]
    fn vertex_stride_sums_semantic_sizes() {
        let mask = SemanticMask::EMPTY
            .with(VertexSemantic::Position)
            .with(VertexSemantic::Normal)
            .with(VertexSemantic::Tangent)
            .with(VertexSemantic::Uv0);
        assert_eq!(mask.vertex_stride(), 12 + 12 + 16 + 8);
    }

    #[test]
    fn sub_mesh_encode_decode_round_trips() {
        let sm = SubMesh {
            first_index: 12,
            index_count: 24,
            material_index: 3,
            flags: 0,
        };
        let bytes = encode_sub_mesh(sm);
        assert_eq!(decode_sub_mesh(&bytes).unwrap(), sm);
    }

    #[test]
    fn aabb_round_trips() {
        let min = [-1.5, 0.0, 2.25];
        let max = [3.5, 1.0, 4.0];
        let bytes = encode_aabb(min, max);
        let (min2, max2) = decode_aabb(&bytes).unwrap();
        assert_eq!(min, min2);
        assert_eq!(max, max2);
    }

    #[test]
    fn semantic_mask_round_trips_through_bits() {
        let mask = SemanticMask::EMPTY
            .with(VertexSemantic::Position)
            .with(VertexSemantic::Color0)
            .with(VertexSemantic::BoneWeights);
        assert_eq!(SemanticMask::from_bits(mask.bits()), mask);
    }
}
