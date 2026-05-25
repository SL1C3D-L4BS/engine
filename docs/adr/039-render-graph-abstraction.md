# ADR-039 — Render-graph abstraction

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 1)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-004 (two-track pipeline), ADR-013 (Determinism
  Contract), ADR-046 (rasterizer oracle regression criteria), ADR-053
  (Phase 5 PR slicing)

## Context

Spec §IV.4.B line 427 states: "The render-graph abstraction
(`engine-render::render_graph`) is designed so passes declare their
inputs and outputs as resources, and the track (A vs B) is a
compile-time selection." The spec gives the *contract* — passes
declare I/O resources, Track A and Track B coexist behind the same
authoring surface — but no ADR yet describes the abstraction's
interface, its resource model, or the oracle-guarantee mechanism by
which Track B is permitted to replace Track A.

Phase 5 PR 1 ships the trait. Phases 9–10+ develop Track B as
research against the Track A oracle. ADR-004 closes the question of
*which* tracks exist; this ADR closes *how they share a surface*.

A naive answer ("a `RenderPass` trait with `inputs()`, `outputs()`,
`record(cb)` methods") is insufficient because the graph also has to:

1. Schedule passes deterministically (Determinism Contract — frame
   hashes must be byte-equal across worker counts and architectures
   when running the simulation path, even though the GPU path itself
   is allowed to use FMA).
2. Detect and emit GPU barriers for read-after-write / write-after-
   read resource transitions automatically — no pass-author should
   ever write a barrier by hand.
3. Permit a pass to declare itself "Track A only", "Track B only", or
   "track-agnostic" so the compile-time track switch
   (`cfg(feature = "track-b")`) cuts the right subset.
4. Surface the oracle invariant: any Track B alternative pass must
   declare the Track A pass it replaces, and pixel-parity is
   verified against it.

## Decision

The render-graph lives in `crates/engine-render/src/render_graph/`.
Four core types and two compile-time switches.

### 1. `Resource` — typed handles into a transient resource heap

```rust
pub struct Resource<T: ResourceType> {
    id: ResourceId,            // dense u32 index, stable per frame
    _phantom: PhantomData<T>,
}

pub trait ResourceType: 'static {
    const KIND: ResourceKind; // Buffer | Texture | Sampler | ...
}
```

Concrete `ResourceType`s: `GBufferAlbedo`, `GBufferNormal`,
`ShadowAtlas`, `LightClusterGrid`, `HdrColorTarget`, `Swapchain`, …
Each is a zero-sized type whose `KIND` carries metadata (format,
extent rule, lifetime). The dense `ResourceId` indices into a
per-frame `ResourcePool` that maps to concrete `wgpu::Texture` /
`wgpu::Buffer` allocations via the `engine-gpu` Level-1 wrapper
(ADR-049).

### 2. `Pass` — the trait every named pass implements

```rust
pub trait Pass: 'static + Send {
    /// Stable name; the telemetry SPAN tag.
    const NAME: &'static str;

    /// Track-A / Track-B / both.
    const TRACK: Track;

    /// Resources read this pass; written-then-read same-pass goes in `writes`.
    fn reads(&self, w: &mut ResourceSet);
    /// Resources written this pass.
    fn writes(&self, w: &mut ResourceSet);

    /// The pass body. Receives a `PassContext` that exposes the
    /// declared resources by typed handle — accessing an undeclared
    /// resource is a compile error (the `PassContext` borrow is
    /// constructed from the declared sets).
    fn record(&mut self, ctx: &mut PassContext);
}
```

The `reads` / `writes` declaration is the same architectural pattern
as `Schedule::add_system_with_access` (ADR-033): the graph relies on
the declarations to schedule, and the runtime backstop is the
pixel-parity oracle, not a borrow checker (the GPU does not stop you
from writing a resource you said you'd only read).

### 3. `RenderGraph` — the topological scheduler

```rust
let mut graph = RenderGraph::new();
graph.add_pass::<GeomFeed>();           // Track::A
graph.add_pass::<Cull>();               // Track::A
graph.add_pass::<DrawOpaque>();         // Track::A
// ... per spec §IV.4.A line 376
graph.compile()?;
graph.execute(world, gpu_device, frame_idx)?;
```

`compile()` performs a stable topological sort of passes by their
declared resource dependencies (`(reads, writes)`). Tie-breaking:
registration order. Within a tied tier (no resource overlap), pass
execution order is non-deterministic across runs but cannot affect
the resulting frame — same proof as ADR-033's parallel scheduler:
declared-disjoint passes cannot observe each other's writes.

`compile()` is also where Track A vs Track B selection happens:
passes with `TRACK == Track::B` are pulled in only when the
`track-b` cargo feature is set. The two tracks are *both* compiled
into the binary by default — switching is a re-`compile()` call,
not a re-link.

### 4. `OracleAlternative` — the Track-B replacement contract

A Track-B pass that replaces a Track-A pass declares it:

```rust
impl Pass for GpuDrivenCullAndDraw {
    const NAME: &'static str = "gpu_driven_cull_and_draw";
    const TRACK: Track = Track::B;
    const REPLACES: &[&'static str] = &["cull", "draw.opaque"];
    // ...
}
```

The `REPLACES` list is consumed by the test harness in
`testbed/engine-raster/`: every Track-B alternative must produce a
pixel-perfect (per ADR-046 thresholds) match against the Track-A
sequence it replaces, on the rasterizer-testbed reference frames.
Until that holds, Track B is opt-in only.

### 5. Compile-time vs runtime track selection

- **Compile-time** (`#[cfg(feature = "track-b")]`): which pass crates
  are linked. Track B research code does not exist in a Track-A-only
  build.
- **Runtime** (`graph.set_track(Track::A | Track::B)`): which of the
  linked passes are scheduled. Lets a single binary host an
  A-vs-B comparison at the rasterizer-testbed CLI level (`engine
  raster --backend gpu` already accepts a track parameter; this ADR
  adds `--track {a,b}`).

## Consequences

- The Phase-5 pass list in spec §IV.4.A line 376 becomes a literal
  registration sequence. Adding a new pass is one `graph.add_pass::<…>()`
  call in the engine's render-loop init.
- Resource lifetime analysis falls out of declared `reads`/`writes`:
  a resource is alive from its first write to its last read. The
  transient `ResourcePool` reuses slots aggressively.
- The trait surface is stable across Phases 5–9. Track-B development
  (Phase 9–10) lands new passes against an unchanged graph API.
- ECS integration: `geom.feed` (the first pass) extracts render data
  from the world. It is a Track::A pass with `writes = [RenderQueue]`;
  the rest of the graph reads `RenderQueue` and never touches the
  ECS. Same isolation rule as the simulation/render boundary in spec
  §IV.2.

## Risks and tradeoffs

- **The trait is generic over `ResourceType`**. Pass-author ergonomics
  depend on a typed-resource macro (`#[derive(Resource)]`) so the
  per-frame handles aren't manually constructed. The macro lives in
  `engine-ecs-macro` (ADR-024); a small `#[render_resource]` attribute
  proc-macro is added there.
- **Stable topological sort tie-breaking by registration order is
  the determinism anchor.** A pass-list reorder is a behavioural
  change even when no other code touches it. This is the same
  contract Bevy 0.18 ships and matches ADR-033 — but it bites the
  first time someone reorders init code "just for readability."
  The CI replay-parity oracle (ADR-033) is the runtime backstop;
  the rasterizer-pixel-parity oracle (ADR-046) is the second.
- **Compile-time track switch + runtime track switch is two knobs
  for one decision.** Justified because PR-time CI wants the
  compile-time form (track-A-only builds stay small) but A-vs-B
  perf comparisons want one binary. The `Track` enum is the single
  source of truth; both knobs consult it.
- **No async / multi-queue support in this iteration.** The graph
  schedules onto a single GPU queue (graphics + compute via the
  same `wgpu::Queue`). Async-compute overlap is a Phase 6+ feature
  per spec §IV.4.A line 385 (the `async.compute` pass for 3DGS).

## Alternatives considered

- **Bevy's render graph (sub-graph + slot model).** Richer than what
  Phase 5 needs; the slot graph is run-time string-keyed and
  introspectable. Rejected: typed resource handles + stable topo
  sort give the same scheduling power with no runtime string
  lookups, and the type-safety surfaces resource-misuse at compile
  time.
- **Per-pass `Box<dyn Pass>` polymorphism.** Allocations per pass
  per frame. Rejected: passes are zero-sized when stateless; stateful
  passes hold their state in their struct and are added once.
- **Inline render code (no graph abstraction).** Phase 5 PR 1 could
  ship without an abstraction and the renderer would still work.
  Rejected: the graph is the contract that lets Track B coexist with
  Track A, and the spec demands it.

## Verification

- The trait compiles and is exercised by the Phase 5 PR 1 software-
  rasterizer integration: the same pass list runs against
  `engine-raster` (CPU) and (in PR 2+) against `engine-gpu` (GPU).
  Pixel-parity oracle ties the two together.
- A `tests/render_graph_topo.rs` integration test (lands with PR 1):
  registers a synthetic 6-pass graph with declared dependencies,
  asserts a stable topological order across {single-thread, 2-worker,
  4-worker, N-worker} graph compilations. Same shape as
  `replay_parity.rs` (ADR-033).
- A `tests/render_graph_track_b_oracle.rs` harness (lands with the
  first Track-B alternative, Phase 9+): for every pass with
  `REPLACES` non-empty, runs the rasterizer-testbed reference scene
  through both pass chains and asserts pixel-parity per ADR-046.
- No CI guard needed: the `wgpu::` boundary guard (ADR-049) is the
  one that keeps render-graph code from reaching past `engine-gpu`.
