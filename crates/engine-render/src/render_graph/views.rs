//! Phase 5.5 A.2b-ii — graph resource resolver (ADR-075 §5).
//!
//! Pass `record()` bodies that issue real GPU work need to translate the
//! abstract [`ResourceId`]s declared via `reads()` / `writes()` into
//! concrete [`engine_gpu`] handles — texture views, buffers, samplers.
//! Phase 5 PR 1 left that translation as a graph-internal TBD; the
//! render graph routed schedule-order through `PassContext::user` and
//! the CPU rasterizer testbed did its own bookkeeping inside that
//! scratchpad. Phase 5.5 A.2b-ii formalises the lookup as a trait
//! [`ResourceResolver`] threaded through [`super::PassContext`].
//!
//! ## Design
//!
//! - **Trait, not concrete struct.** Lets CPU-oracle paths
//!   (`testbed/engine-raster`) pass `None`; lets future test harnesses
//!   inject mock resolvers; lets the production GPU runtime own the
//!   transient pool without the trait knowing.
//! - **Three accessors, one per resource kind.** Match the
//!   [`engine_gpu::BindingResource`] surface so the resolver output
//!   feeds bind-group construction directly. Bindless heap resources
//!   (ADR-044) are NOT routed here — they have their own per-frame
//!   accessor on `PassContext` (to be added when the heap consumer
//!   passes land).
//! - **`TextureView<'_>` by value, `Buffer` / `Sampler` by reference.**
//!   [`engine_gpu::TextureView`] is `Copy`; returning it by value is
//!   ergonomic and matches the pass-`record()` call shape. `Buffer` and
//!   `Sampler` are owned types — references suffice and avoid clone.
//!
//! ## Default implementation
//!
//! [`TransientResourceTable`] is the host-renderer-side default. It
//! owns the per-frame resources and registers each against the
//! [`ResourceId`] the graph builder produced. Production frame loops
//! construct the table, allocate transient textures + buffers +
//! samplers, register them, then pass `&table as &dyn ResourceResolver`
//! into [`super::RenderGraph::execute`].

use engine_gpu::{Buffer, Sampler, Texture, TextureView};

use super::ResourceId;

/// Per-frame transient-resource lookup.
///
/// ADR-075 §5 — pass `record()` bodies consume this to resolve declared
/// [`ResourceId`]s to concrete [`engine_gpu`] handles. Implementors
/// register one resource per `ResourceId` the graph builder produced.
///
/// CPU-oracle paths short-circuit at the [`super::PassContext::resources`]
/// `None` branch and never touch the resolver. Tests that schedule
/// passes without executing them also leave `resources: None` — the
/// resolver only matters at execute time.
///
/// `Debug` is required so [`super::PassContext`] can keep its
/// `#[derive(Debug)]` — the trait-object form needs Debug at the
/// supertrait level to satisfy the derive.
pub trait ResourceResolver: core::fmt::Debug {
    /// Resolve a texture view by id. Returns `None` if the id is not
    /// registered (the pass should short-circuit gracefully).
    fn resolve_view(&self, id: ResourceId) -> Option<TextureView<'_>>;

    /// Resolve a buffer by id.
    fn resolve_buffer(&self, id: ResourceId) -> Option<&Buffer>;

    /// Resolve a sampler by id.
    fn resolve_sampler(&self, id: ResourceId) -> Option<&Sampler>;
}

/// Default [`ResourceResolver`] implementation — a sparse, owned table
/// of per-frame transient resources keyed by [`ResourceId`].
///
/// Built outside the graph by the host renderer (the engine driver,
/// the frame-pacing bench, the parity-fixture harness) and threaded
/// through [`super::RenderGraph::execute`]. The table does NOT allocate
/// resources itself; callers create each [`Texture`] / [`Buffer`] /
/// [`Sampler`] and `register_*` it against the [`ResourceId`] the
/// graph builder handed out.
///
/// ## Lookup cost
///
/// Backed by parallel `Vec<(ResourceId, T)>`s with linear search. The
/// expected per-frame entry count is small (the canonical Phase-5.5
/// graph has ~20 resources; A.3's parity fixtures push it to ~30), and
/// the lookup happens at most once per binding per pass per frame —
/// well within Gregory Ch. 11.5's "preprocess once vs submit each
/// frame" envelope. A hash-map variant ([`engine_core::collections`]
/// per ADR-028) lands as a Phase 6+ optimisation if measurement
/// justifies it.
#[derive(Debug, Default)]
pub struct TransientResourceTable {
    textures: Vec<(ResourceId, Texture)>,
    buffers: Vec<(ResourceId, Buffer)>,
    samplers: Vec<(ResourceId, Sampler)>,
}

impl TransientResourceTable {
    /// Empty table. Same as [`Default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a texture under `id`. Subsequent [`Self::resolve_view`]
    /// calls for `id` return the texture's default view.
    ///
    /// Panics if `id` is already registered against another texture
    /// (the registration loop is at frame-build time, not the hot path
    /// — fail-loud is the right discipline).
    pub fn register_texture(&mut self, id: ResourceId, texture: Texture) {
        assert!(
            !self.textures.iter().any(|(i, _)| *i == id),
            "transient pool: texture id {id:?} already registered"
        );
        self.textures.push((id, texture));
    }

    /// Register a buffer under `id`.
    pub fn register_buffer(&mut self, id: ResourceId, buffer: Buffer) {
        assert!(
            !self.buffers.iter().any(|(i, _)| *i == id),
            "transient pool: buffer id {id:?} already registered"
        );
        self.buffers.push((id, buffer));
    }

    /// Register a sampler under `id`.
    pub fn register_sampler(&mut self, id: ResourceId, sampler: Sampler) {
        assert!(
            !self.samplers.iter().any(|(i, _)| *i == id),
            "transient pool: sampler id {id:?} already registered"
        );
        self.samplers.push((id, sampler));
    }

    /// Count of registered textures.
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Count of registered buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Count of registered samplers.
    pub fn sampler_count(&self) -> usize {
        self.samplers.len()
    }

    /// Discard every registered resource. Called between frames when
    /// the renderer recycles its transient pool wholesale.
    pub fn clear(&mut self) {
        self.textures.clear();
        self.buffers.clear();
        self.samplers.clear();
    }
}

impl ResourceResolver for TransientResourceTable {
    fn resolve_view(&self, id: ResourceId) -> Option<TextureView<'_>> {
        self.textures
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, t)| t.default_view())
    }

    fn resolve_buffer(&self, id: ResourceId) -> Option<&Buffer> {
        self.buffers.iter().find(|(i, _)| *i == id).map(|(_, b)| b)
    }

    fn resolve_sampler(&self, id: ResourceId) -> Option<&Sampler> {
        self.samplers.iter().find(|(i, _)| *i == id).map(|(_, s)| s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty table resolves nothing. The pass-record short-circuit
    /// behaviour relies on `resolve_*` returning `None` for unregistered
    /// ids, not panicking.
    #[test]
    fn empty_table_resolves_to_none() {
        let table = TransientResourceTable::new();
        assert!(table.resolve_buffer(ResourceId(0)).is_none());
        assert!(table.resolve_sampler(ResourceId(0)).is_none());
        assert!(table.resolve_view(ResourceId(0)).is_none());
    }

    /// Counts track registration. Exercised by the A.3 fixture harness
    /// to assert the right number of resources were prepared.
    #[test]
    fn counts_start_at_zero() {
        let table = TransientResourceTable::new();
        assert_eq!(table.texture_count(), 0);
        assert_eq!(table.buffer_count(), 0);
        assert_eq!(table.sampler_count(), 0);
    }

    /// `clear` resets the table to empty. Exercised between frames.
    #[test]
    fn clear_resets_counts() {
        let mut table = TransientResourceTable::new();
        // Without a real device we can't construct Buffer/Texture/Sampler,
        // but we can verify clear() is callable on an empty table.
        table.clear();
        assert_eq!(table.texture_count(), 0);
        assert_eq!(table.buffer_count(), 0);
        assert_eq!(table.sampler_count(), 0);
    }
}
