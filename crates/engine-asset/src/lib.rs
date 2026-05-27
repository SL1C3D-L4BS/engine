//! `engine-asset` — the content-addressed asset pipeline core.
//!
//! Level 1 crate. See `ENGINE_SPECIFICATION_v2.0.md` Part IV.1.
//!
//! Every compiled asset is identified by the [`ContentHash`] of its bytes, so
//! the pipeline deduplicates, caches, and delta-patches deterministically
//! (spec IV.8). Assets ship inside [`Pak`] archives; a [`PakSet`] stacks a base
//! pak with Live Ops update paks (newest-first resolution, per-name
//! kill-switch). The runtime loads assets through an [`AssetServer`], which
//! hands out hot-reloadable typed [`Handle`]s. Update paks may be
//! [signed](sign) and verified before mounting.
//!
//! # Modules
//!
//! - [`hash`] — SHA-256 content addressing.
//! - [`store`] — the deduplicating content-addressed blob store.
//! - [`pak`] — pak archives and the overlay/kill-switch [`PakSet`].
//! - [`handle`] — typed handles and the hot-reloading [`AssetServer`].
//! - [`sign`] — Ed25519 pak signing and verification.
//! - [`texture`] — on-disk [`TextureMeta`] header for compressed BC blobs
//!   (ADR-045 §4).
//! - [`mesh`] — on-disk [`MeshMeta`] header for EMSH geometry blobs
//!   (ADR-061 §1).
//! - [`material`] — on-disk [`MaterialMeta`] header for EMAT material
//!   blobs (ADR-061 §2).
//!
//! # Out of scope
//!
//! Format-specific importers other than glTF (FBX, COLLADA, USD) and the
//! versioned `.scn` / `.sav` formats are later phases — they need the
//! concrete component and scene types that do not exist yet. The glTF
//! importer lives in `tools/engine-mesh-import/` per ADR-062 and never
//! shares an address space with the engine runtime.

pub mod handle;
pub mod hash;
pub mod material;
pub mod mesh;
pub mod pak;
pub mod sign;
pub mod store;
pub mod texture;

pub use handle::{Asset, AssetError, AssetServer, Handle};
pub use hash::ContentHash;
pub use material::{
    MATERIAL_META_BYTES, MAX_FACTORS, MAX_TEXTURE_SLOTS, MaterialMeta, SamplerKind,
    TEXTURE_SLOT_BYTES, TextureSemantic, TextureSlot, decode_texture_slot, encode_texture_slot,
};
pub use mesh::{
    AABB_BYTES, IndexFormat, MAX_SUB_MESHES, MESH_META_BYTES, MeshMeta, SUB_MESH_BYTES,
    SemanticMask, SubMesh, VertexSemantic, decode_aabb, decode_sub_mesh, encode_aabb,
    encode_sub_mesh,
};
pub use pak::{Pak, PakBuilder, PakError, PakSet};
pub use sign::{PakSigner, verify};
pub use store::ContentStore;
pub use texture::{ChannelRole, TEXTURE_META_BYTES, TexExtent, TexFormat, TextureMeta};
