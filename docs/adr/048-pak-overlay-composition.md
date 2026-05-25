# ADR-048 — Pak overlay composition semantics

- Status: Accepted (Phase 5 design contract; implementation lands in
  Phase 5 PR 2 or as a Phase-4½ follow-up — see Verification)
- Date: 2026-05-24
- Phase: 5-prep (asset-layer readiness for Phase 5 shader paks)
- Companion: ADR-008 (content-addressed pipeline), ADR-029 (mmap pak
  loader), Live-Ops scope in spec §XIX.4

## Context

`engine-asset` already exposes a `PakSet` type — composition of
multiple paks under one `AssetServer`. Spec §XIX.4 (Live Ops
Architecture) demands:

> Asset paks compose as overlays. Distribution Platform: deploy pak,
> target segment, monitor opt-in telemetry, kill-switch by asset
> hash. A/B variant delivery: content-addressed, deterministic
> bucketing. Rollback: instant, immutable paks.

The composition *primitive* exists in code. The *semantics* — which
pak's asset wins when multiple paks contain the same `ContentHash`
or the same logical path? what happens when a pak unmounts? — are
unwritten. Phase 5 will ship shader paks (compiled .slang artifacts)
as overlays so the base game and the active mods/DLC layer cleanly;
the semantics need to be pinned before that's safe.

## Decision

### 1. Two-layer addressing — logical path + content hash

Every asset is addressed by two keys:

- **Logical path** (e.g. `shaders/pbr/opaque.slang.bundle`,
  `materials/wood_oak.mat`) — human-meaningful, used by the editor
  and by hot-reload watching.
- **Content hash** (SHA-256 of the bytes, per ADR-008) — the
  invariant identifier for the bytes themselves.

A pak provides a map `logical_path -> ContentHash` (its "manifest")
and a map `ContentHash -> blob` (its "blob store"). The two are
independent: the same bytes can appear under multiple logical paths
across paks; the same logical path can map to different bytes in
different paks (different versions).

### 2. Overlay precedence — explicit layer order

A `PakSet` is an *ordered list* of mounted paks. Mount order is
explicit:

```rust
pakset.mount_base(base_pak)?;
pakset.mount_overlay(dlc1_pak, OverlayPriority::Normal)?;
pakset.mount_overlay(mod_pak, OverlayPriority::High)?;
pakset.mount_overlay(hotfix_pak, OverlayPriority::Critical)?;
```

Resolution order (highest to lowest): Critical → High → Normal →
Base. Within a priority class, last-mounted wins. The order is
deterministic given the mount sequence; `PakSet::resolve(path)`
returns the first hit walking the priority list.

`OverlayPriority::Critical` is reserved for hot-fix paks pushed by
Live Ops (spec §XIX.4 kill-switch flow). The default for user-
authored mods is `Normal`.

### 3. Content-hash deduplication across overlays

Two paks containing the same `ContentHash` share one in-memory blob.
The `ContentStore` (already in `engine-asset`) is the deduplicating
layer; `PakSet` resolves logical paths to content hashes, then
fetches the blob through `ContentStore`. Unmounting a pak decrements
the reference count on its content hashes; only when the refcount
reaches zero is the blob freed.

This means a "rollback" pak that re-supplies the original bytes for
an overridden asset costs zero extra memory if the original pak is
also mounted.

### 4. Kill-switch — by content hash, not by logical path

A kill-switch entry blacklists a *ContentHash*: that blob will never
be served, regardless of which pak nominates it. The kill-switch
list lives in a tiny signed manifest delivered separately
(`engine_asset::KillSwitchManifest`, ed25519-signed per ADR-025).
When a kill-switch hit is detected during pak mount, the
corresponding logical paths fall through to the next overlay (or
fail to resolve, surfaced as `AssetError::KillSwitched`).

Hash-not-path is the correct grain because content-addressing
guarantees uniqueness. Killing by logical path would silently mask
the kill on rename / re-pack; killing by hash is impossible to
evade.

### 5. Eviction — pak unmount semantics

Unmounting a pak:

1. Removes its entry from the `PakSet` mount list.
2. Decrements the refcount on every blob it contributed.
3. Notifies the `AssetServer`: any live `Handle<T>` whose underlying
   bytes came *only* from the unmounting pak now points at the
   "missing asset" fallback (e.g. ADR-044 slot 0 for textures).
4. Fires `Event { "asset.pak_unmounted", Subsystem::Asset, ... }`
   telemetry signal.

A live `Handle<T>` *never crashes* on unmount; it gracefully degrades
to the fallback. Hot code paths that need to detect this can check
`Handle::is_fallback()` (a cheap pointer comparison).

### 6. Version conflict resolution

When two paks at the same priority contain *different* content
hashes for the same logical path, the resolution rule (last-mounted
wins) handles it deterministically. The diagnostics layer logs an
`Event { "asset.path_conflict", ... }` so artists / level designers
can audit unintended overrides. The behaviour is *not* an error —
overlay-based override is the whole point.

### 7. Hot-reload integration

`AssetServer::reload(path)` re-resolves the logical path through the
current `PakSet`. If the resolution result differs (a new pak got
mounted), the handle's underlying bytes change and the file-watcher
emits a reload event. This is how a Live-Ops hotfix lands: the
delivery system mounts the hotfix pak with `OverlayPriority::Critical`,
the asset server detects the change, every Handle pointing at the
patched asset reloads.

## Consequences

- The `PakSet` type gains explicit ordering semantics in its API
  surface; today's `PakSet::add` becomes
  `PakSet::mount_overlay(pak, priority)` with the
  `OverlayPriority` enum.
- Three new error variants on `AssetError`: `PakConflict` (info-only,
  not actually returned), `KillSwitched`, `Unmounted`.
- The asset pak format (already content-addressed) gains a
  `KillSwitchManifest` companion format — separate from the pak,
  delivered by the Live Ops channel. Phase 9+ implements the
  delivery (out of Phase 5 scope); this ADR pins the in-engine
  consumption semantics so the format can be designed cleanly.
- Phase 5's shader paks slot in seamlessly: base pak ships the
  default shader bundles; per-project overlays override them.

## Risks and tradeoffs

- **Last-mounted-wins within a priority class is order-sensitive.**
  Mount order is project config; documented in the spec and in the
  editor. Acceptable: explicit and reviewable.
- **Kill-switch by hash requires the engine to know the kill list at
  mount time.** Kill-switch delivery must precede pak mount in the
  startup sequence. Documented in the runbook.
- **Live `Handle<T>` → fallback on unmount is silent if not
  observed.** The telemetry event surfaces it, but a game that
  unmounts a critical pak mid-frame produces visual fallback flash.
  Mitigation: unmounting is sequenced at frame boundaries (the
  `AssetServer` defers unmount processing to the start of the next
  frame), avoiding mid-frame state changes.
- **Conflicts logged but not errored** can hide a misconfiguration.
  Mitigated by a `engine asset audit` CLI command (Phase 10) that
  produces a static report of every logical-path collision across
  the configured pak set.

## Alternatives considered

- **First-mounted-wins.** Used by some engines (e.g. Quake style).
  Rejected: a hot-fix pak mounted later cannot override.
- **Implicit priority by file-modification-time.** Convenient for
  modders, opaque for shipped products. Rejected.
- **Per-asset whitelist of permissible overrides.** Too restrictive
  for a moddable engine; the spec emphasizes user mods (§XIX.4).
- **Kill-switch by logical path.** Trivially evaded by rename;
  rejected per (4) rationale.

## Verification

- The current `PakSet` already supports mount; the explicit priority
  ordering needs an API change. Two options:
  1. **Land as a Phase-4½ follow-up.** Decoupled from Phase 5
     render work; smaller PR.
  2. **Land in Phase 5 PR 2.** Bundled with the engine-gpu work
     because shader paks are the first user of overlay semantics.
  Recommended: option 1 (Phase-4½). Cleaner separation; the asset
  layer can be tested without touching render.
- Tests (lands with the implementation PR, whichever option chosen):
  - `tests/pak_overlay_precedence.rs`: mount base + overlay; assert
    overlay wins; reverse mount order, assert the other wins.
  - `tests/pak_unmount_handle.rs`: mount, take handle, unmount;
    assert `Handle::is_fallback()` returns true and a telemetry
    event was fired.
  - `tests/pak_kill_switch.rs`: build a kill-switch manifest;
    mount a pak containing the killed hash; assert
    `AssetError::KillSwitched` on read.
  - `tests/pak_dedupe_refcount.rs`: mount two paks with overlapping
    content hashes; assert one allocation in `ContentStore`; unmount
    one, assert allocation persists; unmount second, assert freed.
- Telemetry: `COUNTER "asset.pak_mounted_total"`,
  `COUNTER "asset.pak_unmounted_total"`,
  `EVENT "asset.path_conflict"`, `EVENT "asset.pak_unmounted"`,
  `EVENT "asset.kill_switch_hit"`.
