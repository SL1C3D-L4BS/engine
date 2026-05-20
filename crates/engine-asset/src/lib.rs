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
//!
//! # Out of scope
//!
//! Format-specific importers (glTF, PNG, Slang) and the versioned `.scn` /
//! `.sav` formats are later phases — they need the concrete component and
//! scene types that do not exist yet.

pub mod handle;
pub mod hash;
pub mod pak;
pub mod sign;
pub mod store;

pub use handle::{Asset, AssetError, AssetServer, Handle};
pub use hash::ContentHash;
pub use pak::{Pak, PakBuilder, PakError, PakSet};
pub use sign::{PakSigner, verify};
pub use store::ContentStore;
