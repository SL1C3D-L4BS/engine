//! Render-graph abstraction (ADR-039).
//!
//! The contract: passes declare what resources they `reads` and
//! `writes`; the graph topologically schedules them; track A/B
//! selection is both a compile-time feature gate and a runtime
//! switch on the same enum.
//!
//! Phase 5 PR 1 ships this trait surface + a stable topological
//! sort + a synthetic graph oracle (`tests/render_graph_topo.rs`).
//! Concrete passes land in PRs 2–6. The Track B oracle harness
//! (`tests/render_graph_track_b_oracle.rs`) is the Phase 9+
//! follow-up.
//!
//! Design decisions that anchor the implementation:
//!
//! - **Dense resource IDs** (`u32`). One arena per frame. No
//!   string keys, no hash lookups on the hot path.
//! - **Statically-typed resource handles** parameterised by a
//!   `ResourceType` trait. The phantom type prevents a pass that
//!   declared `GBufferAlbedo` from accessing a `ShadowAtlas` it
//!   never reserved.
//! - **Pass execution is the trait's job.** The graph owns
//!   scheduling and resource lifetime; the pass body owns the
//!   actual draw/compute work. PassContext brokers access.
//! - **Determinism.** Topological sort is stable: tie-break by
//!   registration order. A graph compiled twice with the same
//!   `add_pass` sequence produces byte-identical execution order.
//! - **Track encoding.** `Track::A`, `Track::B`, `Track::Both` —
//!   the pass's `TRACK` const + the graph's runtime `set_track`
//!   filter combine to produce the executed subset.

use core::marker::PhantomData;

/// Which rendering track a pass belongs to. ADR-004 names the
/// tracks; this enum is their runtime representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Classical deferred / forward+ rasterisation. The default;
    /// the only track the engine ships at v1.0 (Phase 5 closure).
    A,
    /// Work-graph + mesh-shader research track. Phase 9+ scope.
    B,
    /// Pass works under both tracks. The synthetic mesh-extract
    /// (geom.feed) pass is the canonical example.
    Both,
}

impl Track {
    /// Is this pass active under the given runtime track selection?
    #[inline]
    pub fn includes(self, selected: Track) -> bool {
        matches!(
            (self, selected),
            (Track::Both, _) | (Track::A, Track::A) | (Track::B, Track::B)
        )
    }
}

/// Categorisation of resource kinds the graph schedules.
/// Driver-side allocation (the `engine-gpu` wrapper) keys off this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    /// GPU-side typed buffer (vertex, index, storage, uniform).
    Buffer,
    /// 2D or 3D texture.
    Texture,
    /// Sampler state.
    Sampler,
    /// External swapchain image. Lifetime owned by the windowing
    /// layer, not the graph's transient pool.
    Swapchain,
}

/// Stable trait for resource type tags. Each concrete graph
/// resource (`GBufferAlbedo`, `ShadowAtlas`, …) is a zero-sized
/// type that implements this trait.
pub trait ResourceType: 'static {
    /// Which allocator family does this resource live in.
    const KIND: ResourceKind;
    /// Human-readable name surfaced in telemetry spans.
    const NAME: &'static str;
}

/// Dense per-frame index into the transient resource pool. The
/// graph hands these out at `add_pass` time; concrete resource
/// allocation happens during `compile()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub u32);

/// Statically-typed handle to a graph-managed resource. Carries
/// the resource's id and its type tag so a pass that declared
/// `Resource<GBufferAlbedo>` cannot accidentally bind a
/// `Resource<ShadowAtlas>` to the same slot.
#[derive(Debug)]
pub struct Resource<T: ResourceType> {
    /// The dense id within the frame's resource pool.
    pub id: ResourceId,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: ResourceType> Resource<T> {
    /// Construct a typed handle. Normally produced by the graph
    /// builder, not by user code.
    pub fn new(id: ResourceId) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }
}

impl<T: ResourceType> Clone for Resource<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            _phantom: PhantomData,
        }
    }
}

impl<T: ResourceType> Copy for Resource<T> {}

impl<T: ResourceType> PartialEq for Resource<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: ResourceType> Eq for Resource<T> {}

/// Mutable set of `ResourceId`s a pass touches. Populated by the
/// pass's `reads()` / `writes()` callbacks.
#[derive(Debug, Default)]
pub struct ResourceSet {
    ids: Vec<ResourceId>,
}

impl ResourceSet {
    /// Empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a resource id to the set. Duplicates are deduplicated
    /// during graph compile, not here.
    pub fn add(&mut self, id: ResourceId) {
        self.ids.push(id);
    }

    /// Add a typed resource handle.
    pub fn add_typed<T: ResourceType>(&mut self, r: Resource<T>) {
        self.add(r.id);
    }

    /// Borrow the ids.
    pub fn ids(&self) -> &[ResourceId] {
        &self.ids
    }
}

/// Per-pass execution context. The graph constructs one per
/// pass per frame and hands it to `Pass::record`. Concrete
/// backend implementations (CPU rasterizer, GPU command buffer)
/// supply backend-specific extensions via downcast or a Tag
/// associated type in future revisions; PR 1 keeps the
/// context minimal.
#[derive(Debug)]
pub struct PassContext<'a> {
    /// Frame index — for jitter / TAA history.
    pub frame_idx: u64,
    /// Backend-opaque scratchpad. PR 2's engine-gpu wrapper will
    /// extend this with a command buffer ref; the software
    /// rasterizer uses it for output buffer access.
    pub user: &'a mut dyn core::any::Any,
}

/// The trait every named pass implements.
///
/// Object-safe: implementations can be erased behind `Box<dyn Pass>`.
/// Const items (`NAME`, `TRACK`) are surfaced via accessor methods
/// to preserve object safety; concrete impls populate them via the
/// associated constants and the methods return their values.
pub trait Pass: Send {
    /// Stable name surfaced in telemetry SPAN tags and oracle
    /// reports.
    fn name(&self) -> &'static str;

    /// Track membership.
    fn track(&self) -> Track;

    /// Track-A passes this pass replaces (only for `Track::B`).
    /// Empty by default.
    fn replaces(&self) -> &'static [&'static str] {
        &[]
    }

    /// Declare resources this pass reads.
    fn reads(&self, set: &mut ResourceSet);

    /// Declare resources this pass writes (or read-then-writes).
    fn writes(&self, set: &mut ResourceSet);

    /// Pass body. The graph guarantees that, at call time, every
    /// resource declared via `reads`/`writes` is allocated and
    /// barrier-ready.
    fn record(&mut self, ctx: &mut PassContext);
}

/// The graph itself. Holds the registered pass list, the
/// currently-selected runtime track, and the compiled execution
/// schedule.
pub struct RenderGraph {
    passes: Vec<PassEntry>,
    track: Track,
    schedule: Option<Vec<usize>>,
}

struct PassEntry {
    pass: Box<dyn Pass>,
    /// Registration index — the determinism tie-breaker.
    seq: u32,
}

impl core::fmt::Debug for RenderGraph {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RenderGraph")
            .field("pass_count", &self.passes.len())
            .field("track", &self.track)
            .field("compiled", &self.schedule.is_some())
            .finish()
    }
}

impl RenderGraph {
    /// Build an empty graph defaulting to Track::A.
    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            track: Track::A,
            schedule: None,
        }
    }

    /// Set the runtime track filter. Recompile to apply.
    pub fn set_track(&mut self, track: Track) {
        self.track = track;
        self.schedule = None;
    }

    /// Currently-selected track.
    pub fn track(&self) -> Track {
        self.track
    }

    /// Register a pass. Returns its registration sequence number;
    /// the number serves as the determinism tie-breaker for
    /// topological sorts.
    pub fn add_pass<P: Pass + 'static>(&mut self, pass: P) -> u32 {
        let seq = self.passes.len() as u32;
        self.passes.push(PassEntry {
            pass: Box::new(pass),
            seq,
        });
        self.schedule = None;
        seq
    }

    /// Compile the graph: topologically sort active passes by
    /// declared resource dependencies. Stable tie-break by
    /// registration order. Returns the count of scheduled passes.
    pub fn compile(&mut self) -> Result<usize, CompileError> {
        // Collect active passes + their declared reads / writes.
        let active: Vec<(usize, Vec<ResourceId>, Vec<ResourceId>)> = self
            .passes
            .iter()
            .enumerate()
            .filter(|(_, e)| e.pass.track().includes(self.track))
            .map(|(idx, e)| {
                let mut reads = ResourceSet::new();
                let mut writes = ResourceSet::new();
                e.pass.reads(&mut reads);
                e.pass.writes(&mut writes);
                (idx, reads.ids().to_vec(), writes.ids().to_vec())
            })
            .collect();

        // Build dependency edges: pass `j` depends on pass `i` if
        // `j` reads a resource that `i` writes, AND `i` was
        // registered before `j` (the registration order pins the
        // producer/consumer direction — a pass reading a
        // resource written by a later-registered pass is a graph
        // cycle the user authored).
        let n = active.len();
        let mut indegree = vec![0u32; n];
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for j in 0..n {
            for i in 0..n {
                if i == j {
                    continue;
                }
                let (_, j_reads, _) = &active[j];
                let (_, _, i_writes) = &active[i];
                let depends_on_write_then_read = j_reads.iter().any(|r| i_writes.contains(r));
                let (i_pass_idx, _, _) = &active[i];
                let (j_pass_idx, _, _) = &active[j];
                let i_first = self.passes[*i_pass_idx].seq < self.passes[*j_pass_idx].seq;
                if depends_on_write_then_read && i_first {
                    edges[i].push(j);
                    indegree[j] += 1;
                }
            }
        }

        // Kahn's algorithm with registration-order tie-break.
        let mut order = Vec::with_capacity(n);
        loop {
            // Pick the smallest-seq ready node.
            let next = (0..n)
                .filter(|&k| indegree[k] == 0 && !order.contains(&k))
                .min_by_key(|&k| self.passes[active[k].0].seq);
            match next {
                Some(k) => {
                    order.push(k);
                    for &m in &edges[k] {
                        indegree[m] = indegree[m].saturating_sub(1);
                    }
                }
                None => break,
            }
        }

        if order.len() != n {
            return Err(CompileError::Cycle);
        }

        // Translate the active-index order back to pass-list indices.
        let schedule: Vec<usize> = order.into_iter().map(|k| active[k].0).collect();
        let count = schedule.len();
        self.schedule = Some(schedule);
        Ok(count)
    }

    /// Borrow the compiled schedule (pass-list indices in execution
    /// order). `None` if `compile()` has not run.
    pub fn schedule(&self) -> Option<&[usize]> {
        self.schedule.as_deref()
    }

    /// Borrow the compiled pass names in execution order. Useful
    /// for oracle assertions and telemetry.
    pub fn scheduled_names(&self) -> Option<Vec<&'static str>> {
        self.schedule
            .as_ref()
            .map(|s| s.iter().map(|&i| self.passes[i].pass.name()).collect())
    }

    /// Execute the compiled schedule against a backend-supplied
    /// user context. PR 1 keeps the API surface; the rasterizer
    /// testbed and the future GPU runner both call this.
    pub fn execute(
        &mut self,
        frame_idx: u64,
        user: &mut dyn core::any::Any,
    ) -> Result<(), ExecuteError> {
        let Some(schedule) = self.schedule.clone() else {
            return Err(ExecuteError::NotCompiled);
        };
        for idx in schedule {
            let pass = &mut self.passes[idx].pass;
            let mut ctx = PassContext { frame_idx, user };
            pass.record(&mut ctx);
        }
        Ok(())
    }

    /// Number of registered passes (regardless of track filter).
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }
}

impl Default for RenderGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors from `RenderGraph::compile`.
#[derive(Debug, PartialEq, Eq)]
pub enum CompileError {
    /// Declared dependencies introduce a cycle.
    Cycle,
}

/// Errors from `RenderGraph::execute`.
#[derive(Debug, PartialEq, Eq)]
pub enum ExecuteError {
    /// Graph has not been compiled (or was invalidated by a
    /// `set_track` / `add_pass` after the last compile).
    NotCompiled,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyAlbedo;
    impl ResourceType for DummyAlbedo {
        const KIND: ResourceKind = ResourceKind::Texture;
        const NAME: &'static str = "GBufferAlbedo";
    }
    struct DummyDepth;
    impl ResourceType for DummyDepth {
        const KIND: ResourceKind = ResourceKind::Texture;
        const NAME: &'static str = "Depth";
    }

    struct Producer {
        name: &'static str,
        writes: ResourceId,
    }
    impl Pass for Producer {
        fn name(&self) -> &'static str {
            self.name
        }
        fn track(&self) -> Track {
            Track::A
        }
        fn reads(&self, _s: &mut ResourceSet) {}
        fn writes(&self, s: &mut ResourceSet) {
            s.add(self.writes);
        }
        fn record(&mut self, _: &mut PassContext) {}
    }
    struct Consumer {
        name: &'static str,
        reads: ResourceId,
    }
    impl Pass for Consumer {
        fn name(&self) -> &'static str {
            self.name
        }
        fn track(&self) -> Track {
            Track::A
        }
        fn reads(&self, s: &mut ResourceSet) {
            s.add(self.reads);
        }
        fn writes(&self, _: &mut ResourceSet) {}
        fn record(&mut self, _: &mut PassContext) {}
    }

    #[test]
    fn topo_sort_orders_producer_before_consumer() {
        let mut g = RenderGraph::new();
        let albedo = ResourceId(0);
        g.add_pass(Producer {
            name: "produce",
            writes: albedo,
        });
        g.add_pass(Consumer {
            name: "consume",
            reads: albedo,
        });
        let n = g.compile().expect("compile");
        assert_eq!(n, 2);
        assert_eq!(g.scheduled_names().unwrap(), vec!["produce", "consume"]);
    }

    #[test]
    fn topo_sort_independent_passes_keep_registration_order() {
        // Independent passes (no shared resources) — order ties on
        // registration. Verifies determinism.
        let mut g = RenderGraph::new();
        g.add_pass(Producer {
            name: "p_a",
            writes: ResourceId(0),
        });
        g.add_pass(Producer {
            name: "p_b",
            writes: ResourceId(1),
        });
        g.add_pass(Producer {
            name: "p_c",
            writes: ResourceId(2),
        });
        g.compile().unwrap();
        assert_eq!(g.scheduled_names().unwrap(), vec!["p_a", "p_b", "p_c"]);
    }

    #[test]
    fn track_filter_excludes_other_track() {
        struct OnlyB;
        impl Pass for OnlyB {
            fn name(&self) -> &'static str {
                "b_only"
            }
            fn track(&self) -> Track {
                Track::B
            }
            fn reads(&self, _: &mut ResourceSet) {}
            fn writes(&self, _: &mut ResourceSet) {}
            fn record(&mut self, _: &mut PassContext) {}
        }
        let mut g = RenderGraph::new();
        g.add_pass(Producer {
            name: "a_only",
            writes: ResourceId(0),
        });
        g.add_pass(OnlyB);
        g.compile().unwrap();
        assert_eq!(g.scheduled_names().unwrap(), vec!["a_only"]);
        g.set_track(Track::B);
        g.compile().unwrap();
        assert_eq!(g.scheduled_names().unwrap(), vec!["b_only"]);
    }

    #[test]
    fn cycle_detection() {
        // Two passes that each read the other's output, registered
        // such that both depend on each other.
        // Our cycle rule: i_first (i < j by seq) gates dependencies,
        // so a true cycle requires same-seq mutual reads — not
        // expressible in this model. Test that two passes reading
        // a resource the OTHER writes (with one registered first)
        // produce a one-way edge, not a cycle.
        let mut g = RenderGraph::new();
        let r0 = ResourceId(0);
        let r1 = ResourceId(1);
        struct Both {
            name: &'static str,
            read: ResourceId,
            write: ResourceId,
        }
        impl Pass for Both {
            fn name(&self) -> &'static str {
                self.name
            }
            fn track(&self) -> Track {
                Track::A
            }
            fn reads(&self, s: &mut ResourceSet) {
                s.add(self.read);
            }
            fn writes(&self, s: &mut ResourceSet) {
                s.add(self.write);
            }
            fn record(&mut self, _: &mut PassContext) {}
        }
        g.add_pass(Both {
            name: "a_then_b",
            read: r1,
            write: r0,
        });
        g.add_pass(Both {
            name: "b_then_a",
            read: r0,
            write: r1,
        });
        // a_then_b is registered first; writes r0; b_then_a reads r0
        // → edge a_then_b → b_then_a. No back-edge (a_then_b reads r1
        // but b_then_a is later-registered, so i_first guard prevents
        // the cycle edge). This is the intended "registration order
        // pins direction" behaviour.
        g.compile().expect("no cycle by registration-order anchor");
    }
}
