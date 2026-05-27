//! Round-trip + content-addressing tests for EMSH / EMAT (ADR-061
//! §Verification).

use engine_asset::{
    AABB_BYTES, ContentHash, IndexFormat, MATERIAL_META_BYTES, MESH_META_BYTES, MaterialMeta,
    MeshMeta, SUB_MESH_BYTES, SamplerKind, SemanticMask, SubMesh, TEXTURE_SLOT_BYTES,
    TextureSemantic, TextureSlot, VertexSemantic, encode_aabb, encode_sub_mesh,
    encode_texture_slot,
};

fn cube_mesh() -> (MeshMeta, Vec<u8>) {
    let mask = SemanticMask::EMPTY
        .with(VertexSemantic::Position)
        .with(VertexSemantic::Normal)
        .with(VertexSemantic::Uv0);
    let stride = mask.vertex_stride() as u8;
    let vertex_count: u32 = 24;
    let index_count: u32 = 36;
    let meta = MeshMeta {
        vertex_count,
        index_count,
        vertex_stride: stride,
        index_format: IndexFormat::U16,
        semantic_mask: mask,
        sub_mesh_count: 1,
    };

    // Per-vertex payload: deterministically chosen bytes derived from
    // the vertex index. Real cube vertex data would have specific
    // positions; for round-trip we only need the byte stream to
    // reconstruct identically.
    let mut payload = Vec::new();
    for v in 0..vertex_count {
        for b in 0..stride {
            payload.push(((v * 31 + u32::from(b)) & 0xff) as u8);
        }
    }
    for i in 0..index_count {
        payload.extend_from_slice(&(i as u16).to_le_bytes());
    }
    payload.extend_from_slice(&encode_sub_mesh(SubMesh {
        first_index: 0,
        index_count,
        material_index: 0,
        flags: 0,
    }));
    payload.extend_from_slice(&encode_aabb([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]));
    (meta, payload)
}

#[test]
fn emsh_round_trips_via_pak_blob() {
    let (meta, payload) = cube_mesh();
    let mut blob = Vec::with_capacity(MESH_META_BYTES + payload.len());
    blob.extend_from_slice(&meta.encode(payload.len() as u32));
    blob.extend_from_slice(&payload);

    let (decoded, payload_out) = MeshMeta::decode(&blob).expect("valid blob");
    assert_eq!(decoded, meta);
    assert_eq!(payload_out, &payload[..]);
}

#[test]
fn emsh_payload_layout_matches_expected_len() {
    let (meta, payload) = cube_mesh();
    let expected = meta.vertex_count as u64 * meta.vertex_stride as u64
        + meta.index_count as u64 * meta.index_format.bytes() as u64
        + meta.sub_mesh_count as u64 * SUB_MESH_BYTES as u64
        + AABB_BYTES as u64;
    assert_eq!(payload.len() as u64, expected);
    assert_eq!(meta.expected_payload_len(), expected);
}

#[test]
fn emsh_identical_bytes_produce_identical_hash() {
    let (meta_a, payload_a) = cube_mesh();
    let (meta_b, payload_b) = cube_mesh();
    let mut blob_a = meta_a.encode(payload_a.len() as u32).to_vec();
    blob_a.extend_from_slice(&payload_a);
    let mut blob_b = meta_b.encode(payload_b.len() as u32).to_vec();
    blob_b.extend_from_slice(&payload_b);

    assert_eq!(ContentHash::of(&blob_a), ContentHash::of(&blob_b));
}

#[test]
fn emsh_one_bit_flip_changes_hash() {
    let (meta, payload) = cube_mesh();
    let mut blob_a = meta.encode(payload.len() as u32).to_vec();
    blob_a.extend_from_slice(&payload);
    let mut blob_b = blob_a.clone();
    // Flip the LSB of the first vertex byte.
    blob_b[MESH_META_BYTES] ^= 0x01;

    assert_ne!(ContentHash::of(&blob_a), ContentHash::of(&blob_b));
}

#[test]
fn emsh_index_format_u32_round_trips() {
    let mask = SemanticMask::EMPTY.with(VertexSemantic::Position);
    let stride = mask.vertex_stride() as u8;
    let meta = MeshMeta {
        vertex_count: 4,
        index_count: 6,
        vertex_stride: stride,
        index_format: IndexFormat::U32,
        semantic_mask: mask,
        sub_mesh_count: 1,
    };
    let mut payload = vec![0u8; meta.expected_payload_len() as usize];
    payload[..AABB_BYTES].copy_from_slice(&encode_aabb([0.0; 3], [1.0; 3]));
    let mut blob = meta.encode(payload.len() as u32).to_vec();
    blob.extend_from_slice(&payload);

    let (decoded, _) = MeshMeta::decode(&blob).expect("valid blob");
    assert_eq!(decoded.index_format, IndexFormat::U32);
    assert_eq!(decoded.expected_payload_len(), payload.len() as u64);
}

fn pbr_material() -> (MaterialMeta, Vec<u8>) {
    let albedo_hash = ContentHash::of(b"cube-albedo");
    let normal_hash = ContentHash::of(b"cube-normal");
    let meta = MaterialMeta {
        shader_id: 0xabad_cafe,
        texture_count: 2,
        factor_count: 4,
    };
    let mut payload = Vec::new();
    payload.extend_from_slice(&encode_texture_slot(TextureSlot {
        semantic: TextureSemantic::Albedo,
        sampler_kind: SamplerKind::Anisotropic,
        content_hash: albedo_hash,
    }));
    payload.extend_from_slice(&encode_texture_slot(TextureSlot {
        semantic: TextureSemantic::Normal,
        sampler_kind: SamplerKind::Linear,
        content_hash: normal_hash,
    }));
    // 4 factors: base color (rgb) + roughness scalar.
    for v in [0.8f32, 0.4, 0.2, 0.5] {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    (meta, payload)
}

#[test]
fn emat_round_trips_via_pak_blob() {
    let (meta, payload) = pbr_material();
    let mut blob = Vec::with_capacity(MATERIAL_META_BYTES + payload.len());
    blob.extend_from_slice(&meta.encode(payload.len() as u32));
    blob.extend_from_slice(&payload);

    let (decoded, payload_out) = MaterialMeta::decode(&blob).expect("valid blob");
    assert_eq!(decoded, meta);
    assert_eq!(payload_out, &payload[..]);
}

#[test]
fn emat_payload_layout_matches_expected_len() {
    let (meta, payload) = pbr_material();
    let expected =
        meta.texture_count as u64 * TEXTURE_SLOT_BYTES as u64 + meta.factor_count as u64 * 4;
    assert_eq!(payload.len() as u64, expected);
    assert_eq!(meta.expected_payload_len(), expected);
}

#[test]
fn emat_identical_bytes_produce_identical_hash() {
    let (meta, payload) = pbr_material();
    let mut a = meta.encode(payload.len() as u32).to_vec();
    a.extend_from_slice(&payload);
    let mut b = meta.encode(payload.len() as u32).to_vec();
    b.extend_from_slice(&payload);

    assert_eq!(ContentHash::of(&a), ContentHash::of(&b));
}

#[test]
fn emat_one_bit_flip_changes_hash() {
    let (meta, payload) = pbr_material();
    let mut a = meta.encode(payload.len() as u32).to_vec();
    a.extend_from_slice(&payload);
    let mut b = a.clone();
    b[MATERIAL_META_BYTES] ^= 0x01;

    assert_ne!(ContentHash::of(&a), ContentHash::of(&b));
}
