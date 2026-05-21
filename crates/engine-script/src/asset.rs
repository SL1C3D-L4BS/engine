//! Asset-pipeline integration for compiled sli modules.
//!
//! A `ScriptModule` is the on-disk byte form of a compiled bytecode
//! [`crate::bytecode::Module`]. Decoding lives behind the engine's
//! `Asset` trait so a script ships through the same content-addressed
//! pak pipeline as every other asset (ADR-008).
//!
//! `BlobSource::Mapped` zero-copy access is preserved: decoding reads
//! the bytes once and produces an owned `Module` — string constants
//! land in `String` rather than borrowing the mmap. Borrowing the
//! mmap directly is a PR-3 follow-up tied to the per-module hot-reload
//! swap.

use crate::bytecode::{Module, decode};

/// A compiled sli module asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptModule {
    /// Decoded bytecode module.
    pub module: Module,
}

impl ScriptModule {
    /// Borrows the underlying bytecode module.
    pub fn module(&self) -> &Module {
        &self.module
    }
}

impl engine_asset::Asset for ScriptModule {
    fn decode(bytes: &[u8]) -> Result<Self, engine_asset::AssetError> {
        let module = decode(bytes).map_err(|e| engine_asset::AssetError::Decode(e.to_string()))?;
        Ok(Self { module })
    }
}
