# ADR-043 — Cluster lights · 16×9×24 binning

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 3)
- Date: 2026-05-24
- Phase: 5 — RENDERING FOUNDATION (Track A)
- Companion: ADR-039 (render graph), ADR-040 (CSM — different
  shadow path), ADR-044 (bindless heap — light list storage)

## Context

Spec §IV.4.A line 381 declares the cluster pass: "Cluster assignment
compute, 16×9×24 tile-slice grid." That fixes the grid dimensions
(16×9 tiles spatially, 24 depth slices) and the pass kind (compute
shader). It does not fix:

- Tile size (16×9 tiles at 1440p → tile_w = 160 px, tile_h = 160 px:
  a moderate tile).
- Depth slice distribution (uniform? logarithmic?).
- Per-cluster light list format (compact bitfield? indexed list with
  count?).
- Maximum lights per cluster.
- Compute shader workgroup geometry.

The clustered-forward / clustered-deferred pattern (Olsson-Persson
2012) is the modern baseline; *Real-Time Rendering* 4th ch. 20
documents the variant choices.

## Decision

### 1. Grid: 16 × 9 × 24, logarithmic depth slices

```
slice_z(i) = z_near · (z_far / z_near)^(i / 24)   for i in 0..=24
```

Logarithmic depth distribution matches perspective; uniform slicing
wastes resolution near the camera. 24 slices covers 0.1 m → 1000 m
with ~2× depth ratio per slice (a reasonable per-slice depth budget
for light culling).

### 2. Per-cluster light list — 8-bit count + 32×16-bit indices

```rust
#[repr(C, align(16))]
struct ClusterCell {
    light_count: u8,          // 0..=32
    _pad: [u8; 3],
    light_indices: [u16; 32], // bindless light ids
}
// sizeof = 4 + 64 = 68 bytes; rounded to 80 (5×16 bytes) for alignment
```

Max 32 lights per cluster. The cell is small enough to live in shared
memory across a workgroup. 16-bit light indices support up to 65 535
lights per scene — far above realistic budgets.

Storage: a single SSBO of `[ClusterCell; 16 * 9 * 24]` = 3 456 cells
× 80 B = ~270 KiB per frame, on the GPU. Bindless (ADR-044).

### 3. Light data — separate SSBO of bindless `Light` records

```rust
#[repr(C, align(16))]
struct GpuLight {
    position: Vec3,
    range: f32,
    color: Vec3,
    intensity: f32,
    light_type: u32,   // 0=point, 1=spot, 2=directional (rare)
    spot_inner_cos: f32,
    spot_outer_cos: f32,
    shadow_atlas_idx: u32, // u32::MAX = no shadow
}
```

The lighting accumulation pass reads `cluster_cells[cell_idx]`,
iterates `light_indices[0..light_count]`, dereferences
`lights[light_idx]`. Standard pattern.

### 4. Cluster-assignment compute pass — workgroup geometry

Workgroup size: 64 threads (`@workgroup_size(64, 1, 1)`). Each
workgroup processes one (tile_x, tile_y) screen tile and walks the
full 24-slice depth column. 16 × 9 tiles = 144 workgroups dispatched
per frame.

Per workgroup, the 64 threads parallelise:
- 32 threads check sphere/AABB intersection against the cluster
  frustum (for point/spot lights).
- The 24 depth slices are walked sequentially; for each slice, the
  intersecting lights are appended to `cluster_cells[slice * 144 +
  workgroup_idx].light_indices[…]`.

Light count clamp: when `light_count == 32`, additional lights are
dropped silently. A `COUNTER "render.cluster_light_overflow"`
telemetry signal records overflow events so artists see when scenes
exceed budget.

### 5. Cluster lookup at fragment shading

In `draw.opaque.2` (lighting accumulation):

```glsl
let tile_xy = floor(frag_screen_pos / tile_size);
let slice = log_slice(frag_view_z);                  // ADR formula
let cell_idx = slice * 144 + tile_xy.y * 16 + tile_xy.x;
let cell = cluster_cells[cell_idx];
var color = Vec3::ZERO;
for i in 0..cell.light_count {
    let light = lights[cell.light_indices[i]];
    color = color + evaluate_brdf(surf, light);
}
```

One indirect read per fragment to find the cell, then at most 32
sequential light evaluations. Cluster-coherent within a screen tile,
so cache-friendly.

## Consequences

- Two render-graph passes register in `light.cluster` (compute) and
  consume in `draw.opaque.2` (graphics). The compute pass
  `writes = [ClusterCells]`; the lighting pass
  `reads = [ClusterCells, Lights]`.
- 270 KiB per-frame for the cluster grid. The light SSBO is ~64 B
  per light × 1024 lights baseline = 64 KiB. Together: ~334 KiB on
  the GPU per frame. Negligible vs. G-buffer + history (≥ 100 MiB).
- 32-light-per-cluster cap is the design envelope. Realistic scenes
  hit it only in pathological cases (a chandelier with many bulbs);
  the telemetry overflow counter surfaces it.
- The cluster grid is one of the items the bindless heap (ADR-044)
  indexes; the lighting pass reads a fixed pair of SSBOs by index.

## Risks and tradeoffs

- **32 lights per cluster is a fixed cap.** Phase 6+ could move to a
  variable-length list with a separate index buffer (Olsson-Persson
  *Practical Clustered Shading*). Adds an indirection per fragment
  light; trade-off uncertain on the RX 580 target.
- **16×9 tiles at 1440p means 160×160 px tiles.** Larger than
  Frostbite's 32×32. The trade-off: bigger tiles → fewer cells to
  cluster-cull (cheap CPU/GPU), more lights per cell (more fragment
  cost). 16×9 was the spec's choice and is reasonable for the RX
  580 fragment budget.
- **Logarithmic slice distribution clusters more lights at the far
  end.** Near-camera lights (the common case) get fine-grained
  binning; far lights cluster heavily. Acceptable: far lights are
  attenuated more anyway.
- **Compute dispatch is 144 workgroups; very small.** RX 580 has
  36 CUs; 4 workgroups per CU. Underfills the GPU for the cluster
  pass alone. Mitigated by overlapping with the SSAO compute pass
  on async-compute (Phase 6+; Phase 5 runs them sequentially).

## Alternatives considered

- **Forward+ (tiled-only, no slices).** Simpler but loses depth
  culling — distant lights leak into near tiles. Rejected for PBR
  scenes with many small lights.
- **Light prepass / Z-prepass + classic deferred lighting.** The
  classic technique; loses cluster culling. Rejected because the
  spec mandates a cluster pass and the perf scales worse with
  light count.
- **GPU-driven cluster updates with persistent threads.** Phase 6+
  research candidate; intersects with Track B (work-graph) plans.

## Verification

- Implementation lands with Phase 5 PR 3. Tests:
  - `tests/cluster_grid_geometry.rs`: deterministic slice-z
    distribution matches the closed-form expression for a known
    frustum.
  - `tests/cluster_assignment_oracle.rs`: synthetic 100-light scene
    on a known camera; CPU reference cluster assignment (in
    `engine-raster`) vs GPU output. Per-cell light-id sets must
    match (order may differ — sets, not sequences).
  - `tests/cluster_pixel_parity.rs`: rasterizer-testbed reference
    scene; CPU lighting accumulation vs GPU within ADR-046 threshold.
- Telemetry:
  - `SPAN("light.cluster", Subsystem::Render)` per frame.
  - `COUNTER "render.cluster_light_overflow"` increments when a
    cluster cell would exceed 32 lights.
  - `GAUGE "render.lights_visible"` per frame.
- No CI guard specific to clusters.
