//! Upscaler trait surface — ADR-005 + Phase 5 PR 5.
//!
//! Modern realtime renderers do not present at the resolution they render
//! (spec Part IV.4.A line 406, ADR-005 §Context). The renderer asks an
//! [`UpscalerProvider`] for a final-resolution buffer; the provider owns
//! the choice of algorithm.
//!
//! Phase 5 PR 5 lands the trait + the registry + four registered
//! providers. Three vendor wrappers ([`VendorDlss`], [`VendorFsr`],
//! [`VendorXess`]) ship as `supports() = false` stubs — the real SDK
//! bindings land in Phase 6 per ADR-005 §Consequences. The owned
//! [`OwnedBilinear`] placeholder always reports support; the actual
//! pixel math is the CPU oracle in `engine_raster::upscale`. The owned
//! ONNX temporal upscaler is Phase 6+ per spec line 1634.
//!
//! Selection (ADR-005 §Decision) is "vendor first, then best match,
//! then owned." [`UpscalerRegistry::select`] walks its providers in
//! priority order; the first whose `supports()` returns `true` wins.
//! With all vendor stubs returning `false` in PR 5 the bilinear
//! placeholder is selected on every host — exactly the behaviour the
//! oracle and the milestone bench expect.
//!
//! The chosen provider is reported via a caller-supplied
//! [`SelectionLogger`] callback. Engine-render avoids a hard dependency
//! on `engine-telemetry` (Level 1 ↔ Level 2 coupling); the bench binary
//! and the future renderer wire this up to the telemetry channel
//! (ADR-010).

use engine_gpu::Device;

/// Vendor identity for the registered upscalers. Used by the registry
/// for priority ordering and by the telemetry channel for the chosen-
/// provider span tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscalerKind {
    /// NVIDIA DLSS (Streamline SDK, Phase 6 binding).
    Dlss,
    /// AMD FSR 4 (Phase 6 binding).
    Fsr,
    /// Intel XeSS 2 (Phase 6 binding).
    Xess,
    /// Owned bilinear placeholder. Phase 5 PR 5 ships this so the trait
    /// is end-to-end testable; the owned ONNX temporal upscaler replaces
    /// it in Phase 6+ per ADR-005 §Consequences.
    OwnedBilinear,
    /// Owned ONNX temporal upscaler. Phase 6+ deliverable; the trait
    /// surface reserves the discriminant now.
    OwnedOnnx,
}

impl UpscalerKind {
    /// Human-readable name surfaced in telemetry spans and bench JSON.
    pub fn name(self) -> &'static str {
        match self {
            UpscalerKind::Dlss => "vendor.dlss",
            UpscalerKind::Fsr => "vendor.fsr",
            UpscalerKind::Xess => "vendor.xess",
            UpscalerKind::OwnedBilinear => "owned.bilinear",
            UpscalerKind::OwnedOnnx => "owned.onnx",
        }
    }
}

/// Per-frame upscaler invocation context.
///
/// The renderer fills this at the upscale pass's `record()` time. The
/// `user` slot is the backend-opaque scratchpad — the CPU oracle plumbs
/// pixel buffers; the GPU path (Phase 6) plumbs a command encoder + the
/// input / output texture views.
pub struct UpscaleCtx<'a> {
    /// Frame counter — TAA history identification, jitter cross-check.
    pub frame_idx: u64,
    /// Sub-pixel jitter the renderer applied for this frame (matches
    /// `engine_raster::post_fx::jitter_for_frame`).
    pub jitter: [f32; 2],
    /// Internal render resolution (the input to the upscaler).
    pub input_extent: [u32; 2],
    /// Final display resolution (the output).
    pub output_extent: [u32; 2],
    /// Backend-opaque scratchpad. The CPU oracle uses
    /// `&mut UpscaleCpuBuffers`; the GPU runner will use a struct
    /// carrying a [`engine_gpu::CommandEncoder`] handle + bindless ids.
    pub user: &'a mut dyn core::any::Any,
}

/// Successful upscaler return token.
///
/// The trait deliberately does not return the upscaled buffer here — the
/// backend writes it through the `user` scratchpad and the renderer
/// reads it through the graph's `UpscaledColor` resource handle (see
/// [`UpscalePass`](crate::passes::UpscalePass)). This keeps the trait
/// surface allocator-free and backend-agnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpscaleResult {
    /// Which provider produced the output (always equals the invoked
    /// provider's [`UpscalerProvider::kind`]).
    pub kind: UpscalerKind,
    /// The output resolution the provider actually wrote, in case the
    /// vendor SDK rounded or rescaled. PR 5's stubs and the bilinear
    /// placeholder return `ctx.output_extent` unchanged.
    pub output_extent: [u32; 2],
}

/// Failure surface for [`UpscalerProvider::upscale`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpscaleError {
    /// The provider declared support but failed to produce an output.
    /// Reserved for runtime conditions (driver lost, SDK out-of-memory).
    /// PR 5 vendor stubs never return this — they refuse at `supports()`.
    Internal,
    /// `ctx.user` did not carry the buffer / encoder shape the provider
    /// expected. Backend-vs-frontend mismatch (a CPU oracle invoked the
    /// GPU stub, or vice versa).
    BackendMismatch,
    /// The provider was invoked despite `supports()` returning false.
    /// Strictly a caller error; the registry never produces this for a
    /// provider that passed selection.
    NotSupported,
}

/// The trait every upscaler implementation realises (ADR-005 §Decision).
///
/// Object-safe: the registry stores `Box<dyn UpscalerProvider>`.
pub trait UpscalerProvider: Send + Sync {
    /// Stable kind tag (ADR-010 telemetry stream key).
    fn kind(&self) -> UpscalerKind;

    /// Stable human-readable name. Convenience wrapper around
    /// `kind().name()`; implementations rarely override.
    fn name(&self) -> &'static str {
        self.kind().name()
    }

    /// Does this provider support the given device? Vendor providers
    /// inspect `device.features()` / `device.limits()` and the driver
    /// loader state; the owned placeholder is universal.
    fn supports(&self, device: &Device) -> bool;

    /// Run the upscale for the current frame. Implementations write
    /// the upscaled buffer through `ctx.user` and return a token. PR
    /// 5's bilinear placeholder is a no-op pixel-side — the actual
    /// math is `engine_raster::upscale::bilinear_upscale`, which the
    /// bench binary calls directly.
    fn upscale(&self, ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError>;
}

/// NVIDIA DLSS provider — Phase 6 binding lands the real Streamline SDK
/// integration. PR 5 ships the stub: [`UpscalerProvider::supports`]
/// always returns false so the registry falls through to the next
/// candidate. [`UpscalerProvider::upscale`] therefore never runs in
/// PR-5 selection paths; if called directly it returns
/// [`UpscaleError::NotSupported`].
pub struct VendorDlss;

impl UpscalerProvider for VendorDlss {
    fn kind(&self) -> UpscalerKind {
        UpscalerKind::Dlss
    }
    fn supports(&self, _device: &Device) -> bool {
        // Phase 5 PR 5: stub. Streamline loader detection lives in the
        // Phase 6 binding crate; until then, decline.
        false
    }
    fn upscale(&self, _ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        Err(UpscaleError::NotSupported)
    }
}

/// AMD FSR 4 provider — Phase 6 binding. RDNA 4 tensor path / FSR 3.x
/// spatial fallback are both branched on inside the Phase-6 SDK
/// wrapper, not in this trait. Stubbed identically to [`VendorDlss`].
pub struct VendorFsr;

impl UpscalerProvider for VendorFsr {
    fn kind(&self) -> UpscalerKind {
        UpscalerKind::Fsr
    }
    fn supports(&self, _device: &Device) -> bool {
        false
    }
    fn upscale(&self, _ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        Err(UpscaleError::NotSupported)
    }
}

/// Intel XeSS 2 provider — Phase 6 binding. The XeSS SDK's own feature
/// detection (`xessIsSupported`) will be wired in then. Stubbed.
pub struct VendorXess;

impl UpscalerProvider for VendorXess {
    fn kind(&self) -> UpscalerKind {
        UpscalerKind::Xess
    }
    fn supports(&self, _device: &Device) -> bool {
        false
    }
    fn upscale(&self, _ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        Err(UpscaleError::NotSupported)
    }
}

/// Owned bilinear placeholder (ADR-005 §Decision, last bullet).
///
/// `supports()` is universally `true` — bilinear runs everywhere. The
/// actual pixel math lives in `engine_raster::upscale::bilinear_upscale`
/// so the CPU oracle and the bench binary share a single reference. The
/// trait body here is the no-op token-return that the render graph's
/// [`UpscalePass`](crate::passes::UpscalePass) drives; the math
/// dispatch happens at the backend layer.
pub struct OwnedBilinear;

impl UpscalerProvider for OwnedBilinear {
    fn kind(&self) -> UpscalerKind {
        UpscalerKind::OwnedBilinear
    }
    fn supports(&self, _device: &Device) -> bool {
        true
    }
    fn upscale(&self, ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        Ok(UpscaleResult {
            kind: UpscalerKind::OwnedBilinear,
            output_extent: ctx.output_extent,
        })
    }
}

/// Owned ONNX temporal upscaler (ADR-067 §2). The trait surface
/// reservation Phase 5 PR 5 deferred, now filled.
///
/// `supports()` is stubbed at `false` until the `ort` (ONNX Runtime)
/// binding and the trained `temporal_upscaler_v1.onnx` model bundle
/// land in a follow-up. When activated, this provider becomes the
/// universal-coverage entry in the cascade — it slots above
/// [`OwnedBilinear`] but below the vendor SDKs per ADR-066 §6.
///
/// The Phase 6 PR 5 surface ships the *cascade position* + the trait
/// implementation skeleton so a future `OwnedOnnxTemporal::with_model`
/// constructor can land without rewiring the registry. The
/// `OwnedOnnx` discriminant in [`UpscalerKind`] has been reserved
/// since Phase 5 PR 5.
pub struct OwnedOnnxTemporal;

impl UpscalerProvider for OwnedOnnxTemporal {
    fn kind(&self) -> UpscalerKind {
        UpscalerKind::OwnedOnnx
    }
    fn supports(&self, _device: &Device) -> bool {
        // Stub. ADR-067 §6 states `supports()` should return true
        // whenever the ONNX runtime can initialize; that requires the
        // `ort` binding which lands in a follow-up. Until then the
        // cascade falls through to `OwnedBilinear` on every host —
        // exactly the Phase-5 behaviour, preserved while the
        // discriminant is wired through the registry.
        false
    }
    fn upscale(&self, _ctx: &mut UpscaleCtx<'_>) -> Result<UpscaleResult, UpscaleError> {
        Err(UpscaleError::NotSupported)
    }
}

/// Callback invoked by the registry when a provider is selected. The
/// renderer points this at its `engine_telemetry` channel; the bench
/// binary captures it into the JSON report. Owning the dependency at
/// the call-site keeps `engine-render` free of a telemetry dep.
pub type SelectionLogger<'a> = &'a mut dyn FnMut(UpscalerKind);

/// Ordered priority list of upscaler providers. ADR-005 §Decision
/// fixes the priority — vendor first, best match second, owned last —
/// so the constructor that wires the four stock providers also pins
/// the order.
pub struct UpscalerRegistry {
    providers: Vec<Box<dyn UpscalerProvider>>,
}

impl UpscalerRegistry {
    /// Construct an empty registry. Callers add providers in priority
    /// order via [`UpscalerRegistry::register`]; the helper
    /// [`UpscalerRegistry::with_phase5_defaults`] populates the
    /// PR-5-shipped quartet (DLSS → FSR → XeSS → bilinear).
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Populate the registry with the four PR-5-shipped providers in
    /// ADR-005 priority order: DLSS → FSR → XeSS → OwnedBilinear.
    ///
    /// Superseded by [`UpscalerRegistry::with_phase6_defaults`], which
    /// inserts the [`OwnedOnnxTemporal`] provider between XeSS and
    /// bilinear per ADR-066 §6. Both helpers fall through to
    /// `OwnedBilinear` on every host while the vendor + ORT bindings
    /// remain stubbed.
    #[deprecated(since = "0.3.0", note = "Use `with_phase6_defaults` instead.")]
    pub fn with_phase5_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(VendorDlss));
        r.register(Box::new(VendorFsr));
        r.register(Box::new(VendorXess));
        r.register(Box::new(OwnedBilinear));
        r
    }

    /// Populate the registry with the five Phase-6 providers in
    /// ADR-066 §6 priority order:
    /// DLSS → FSR → XeSS → OwnedOnnxTemporal → OwnedBilinear.
    ///
    /// Until the vendor SDKs link and the `ort` binding lands, the
    /// first four return `supports() == false` and selection falls
    /// through to `OwnedBilinear` on every host — same as the
    /// Phase-5 default, but with the ONNX provider reserved in the
    /// cascade so a future SDK-bringing PR is a binding swap only.
    pub fn with_phase6_defaults() -> Self {
        let mut r = Self::new();
        r.register(Box::new(VendorDlss));
        r.register(Box::new(VendorFsr));
        r.register(Box::new(VendorXess));
        r.register(Box::new(OwnedOnnxTemporal));
        r.register(Box::new(OwnedBilinear));
        r
    }

    /// Populate the registry per a parsed `engine.toml [upscaler]`
    /// block.
    ///
    /// `provider = "auto"` walks the full Phase-6 cascade (identical
    /// to [`UpscalerRegistry::with_phase6_defaults`]). A specific
    /// provider name registers that provider followed by
    /// [`OwnedBilinear`] as the universal fallback — so a host whose
    /// device declines the forced provider still produces a frame
    /// rather than a hard failure. The `quality` field is recorded
    /// for the [`UpscaleCtx`] caller; the registry itself does not
    /// consume it.
    pub fn with_phase6_defaults_from_config(cfg: &crate::upscaler_config::UpscalerConfig) -> Self {
        use crate::upscaler_config::Provider;
        let mut r = Self::new();
        match cfg.provider {
            Provider::Auto => {
                r.register(Box::new(VendorDlss));
                r.register(Box::new(VendorFsr));
                r.register(Box::new(VendorXess));
                r.register(Box::new(OwnedOnnxTemporal));
                r.register(Box::new(OwnedBilinear));
            }
            Provider::Dlss => {
                r.register(Box::new(VendorDlss));
                r.register(Box::new(OwnedBilinear));
            }
            Provider::Fsr => {
                r.register(Box::new(VendorFsr));
                r.register(Box::new(OwnedBilinear));
            }
            Provider::Xess => {
                r.register(Box::new(VendorXess));
                r.register(Box::new(OwnedBilinear));
            }
            Provider::OwnedOnnx => {
                r.register(Box::new(OwnedOnnxTemporal));
                r.register(Box::new(OwnedBilinear));
            }
            Provider::OwnedBilinear => {
                r.register(Box::new(OwnedBilinear));
            }
        }
        r
    }

    /// Append a provider to the priority list.
    pub fn register(&mut self, provider: Box<dyn UpscalerProvider>) {
        self.providers.push(provider);
    }

    /// Number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// True if no providers are registered.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Kinds of the registered providers, in priority order. Useful for
    /// asserting the registry's shape from tests and for the bench
    /// binary's JSON report.
    pub fn kinds(&self) -> Vec<UpscalerKind> {
        self.providers.iter().map(|p| p.kind()).collect()
    }

    /// Pick the first provider whose `supports()` accepts the device.
    /// Returns `None` only when the registry is empty (the bilinear
    /// placeholder is universally supportive, so the stock registry
    /// always selects).
    ///
    /// The chosen provider's kind is also reported through `logger` so
    /// the caller can route it to the telemetry channel (ADR-005
    /// §Decision item 3, ADR-010).
    pub fn select<'a>(
        &'a self,
        device: &Device,
        logger: SelectionLogger<'_>,
    ) -> Option<&'a dyn UpscalerProvider> {
        self.select_with(|p| p.supports(device), logger)
    }

    /// Lower-level selection that takes an arbitrary predicate over the
    /// provider rather than a `&Device`. Production code calls
    /// [`UpscalerRegistry::select`]; tests use this entry point to drive
    /// the cascade without standing up a real `engine_gpu::Device` (which
    /// requires backend features the workspace CI does not enable).
    ///
    /// The first provider for which `predicate(p)` returns `true` is
    /// reported via `logger` and returned. Walks in registration order;
    /// stops at the first match.
    pub fn select_with<'a, F>(
        &'a self,
        mut predicate: F,
        logger: SelectionLogger<'_>,
    ) -> Option<&'a dyn UpscalerProvider>
    where
        F: FnMut(&dyn UpscalerProvider) -> bool,
    {
        for p in &self.providers {
            let r: &dyn UpscalerProvider = p.as_ref();
            if predicate(r) {
                logger(r.kind());
                return Some(r);
            }
        }
        None
    }
}

impl Default for UpscalerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for UpscalerRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UpscalerRegistry")
            .field("len", &self.providers.len())
            .field("kinds", &self.kinds())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // No real [`Device`] is constructed: the workspace `wgpu` dep is
    // configured without backend features (matches `engine-gpu`'s own
    // unit tests), so the selection-path tests live in the bench
    // binary's integration suite. The unit tests here cover the parts
    // of the trait surface that do not depend on the device argument.

    #[test]
    fn kind_names_are_stable() {
        assert_eq!(UpscalerKind::Dlss.name(), "vendor.dlss");
        assert_eq!(UpscalerKind::Fsr.name(), "vendor.fsr");
        assert_eq!(UpscalerKind::Xess.name(), "vendor.xess");
        assert_eq!(UpscalerKind::OwnedBilinear.name(), "owned.bilinear");
        assert_eq!(UpscalerKind::OwnedOnnx.name(), "owned.onnx");
    }

    #[test]
    fn vendor_stubs_invoked_directly_return_not_supported() {
        let mut scratch: u32 = 0;
        let mut ctx = UpscaleCtx {
            frame_idx: 0,
            jitter: [0.0, 0.0],
            input_extent: [1280, 720],
            output_extent: [2560, 1440],
            user: &mut scratch,
        };
        assert_eq!(
            VendorDlss.upscale(&mut ctx),
            Err(UpscaleError::NotSupported)
        );
        assert_eq!(VendorFsr.upscale(&mut ctx), Err(UpscaleError::NotSupported));
        assert_eq!(
            VendorXess.upscale(&mut ctx),
            Err(UpscaleError::NotSupported)
        );
    }

    #[test]
    fn owned_bilinear_returns_output_extent_unchanged() {
        let mut scratch: u32 = 0;
        let mut ctx = UpscaleCtx {
            frame_idx: 42,
            jitter: [0.125, -0.375],
            input_extent: [1280, 720],
            output_extent: [2560, 1440],
            user: &mut scratch,
        };
        let r = OwnedBilinear.upscale(&mut ctx).expect("bilinear succeeds");
        assert_eq!(r.kind, UpscalerKind::OwnedBilinear);
        assert_eq!(r.output_extent, [2560, 1440]);
    }

    #[test]
    #[allow(deprecated)]
    fn registry_phase5_defaults_order_is_dlss_fsr_xess_bilinear() {
        // Surface-level smoke for the deprecated constructor — the
        // existing PR-5 callsite shape continues to compile and yields
        // the original four-provider cascade.
        let r = UpscalerRegistry::with_phase5_defaults();
        assert_eq!(r.len(), 4);
        assert!(!r.is_empty());
        assert_eq!(
            r.kinds(),
            vec![
                UpscalerKind::Dlss,
                UpscalerKind::Fsr,
                UpscalerKind::Xess,
                UpscalerKind::OwnedBilinear,
            ]
        );
    }

    #[test]
    fn registry_phase6_defaults_inserts_onnx_above_bilinear() {
        let r = UpscalerRegistry::with_phase6_defaults();
        assert_eq!(r.len(), 5);
        assert_eq!(
            r.kinds(),
            vec![
                UpscalerKind::Dlss,
                UpscalerKind::Fsr,
                UpscalerKind::Xess,
                UpscalerKind::OwnedOnnx,
                UpscalerKind::OwnedBilinear,
            ]
        );
    }

    #[test]
    fn owned_onnx_temporal_is_stubbed_until_ort_lands() {
        let mut scratch: u32 = 0;
        let mut ctx = UpscaleCtx {
            frame_idx: 0,
            jitter: [0.0, 0.0],
            input_extent: [1280, 720],
            output_extent: [2560, 1440],
            user: &mut scratch,
        };
        assert_eq!(
            OwnedOnnxTemporal.upscale(&mut ctx),
            Err(UpscaleError::NotSupported)
        );
    }

    #[test]
    fn phase6_cascade_falls_through_to_bilinear_until_onnx_supports() {
        // With OwnedOnnxTemporal::supports() returning false the
        // cascade walks DLSS → FSR → XeSS → OwnedOnnx → OwnedBilinear
        // and lands on bilinear. When the `ort` binding lands and the
        // ONNX provider's `supports()` flips to true, this test must
        // change — that's the signal that the cascade behaviour
        // shifted.
        let r = UpscalerRegistry::with_phase6_defaults();
        let mut chosen: Option<UpscalerKind> = None;
        let mut logger_box: Box<dyn FnMut(UpscalerKind)> = Box::new(|k| chosen = Some(k));
        let logger: SelectionLogger<'_> = &mut *logger_box;
        // Predicate mirrors what `supports()` returns: vendor + onnx
        // false, bilinear true.
        let picked = r
            .select_with(|p| matches!(p.kind(), UpscalerKind::OwnedBilinear), logger)
            .expect("bilinear must be selectable");
        assert_eq!(picked.kind(), UpscalerKind::OwnedBilinear);
        drop(logger_box);
        assert_eq!(chosen, Some(UpscalerKind::OwnedBilinear));
    }

    #[test]
    fn registry_new_is_empty() {
        let r = UpscalerRegistry::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert_eq!(r.kinds(), Vec::<UpscalerKind>::new());
    }

    #[test]
    fn registry_register_appends_in_call_order() {
        let mut r = UpscalerRegistry::new();
        r.register(Box::new(OwnedBilinear));
        r.register(Box::new(VendorFsr));
        assert_eq!(
            r.kinds(),
            vec![UpscalerKind::OwnedBilinear, UpscalerKind::Fsr]
        );
    }

    #[test]
    fn registry_from_config_auto_matches_default_cascade() {
        use crate::upscaler_config::{Provider, Quality, UpscalerConfig};
        let cfg = UpscalerConfig {
            provider: Provider::Auto,
            quality: Quality::Balanced,
        };
        let r = UpscalerRegistry::with_phase6_defaults_from_config(&cfg);
        assert_eq!(r.kinds(), UpscalerRegistry::with_phase6_defaults().kinds());
    }

    #[test]
    fn registry_from_config_forces_single_vendor_plus_bilinear() {
        use crate::upscaler_config::{Provider, Quality, UpscalerConfig};
        for (forced, expected_first) in [
            (Provider::Dlss, UpscalerKind::Dlss),
            (Provider::Fsr, UpscalerKind::Fsr),
            (Provider::Xess, UpscalerKind::Xess),
            (Provider::OwnedOnnx, UpscalerKind::OwnedOnnx),
        ] {
            let cfg = UpscalerConfig {
                provider: forced,
                quality: Quality::Balanced,
            };
            let r = UpscalerRegistry::with_phase6_defaults_from_config(&cfg);
            assert_eq!(
                r.kinds(),
                vec![expected_first, UpscalerKind::OwnedBilinear],
                "forced {forced:?}",
            );
        }
    }

    #[test]
    fn registry_from_config_forces_bilinear_only() {
        use crate::upscaler_config::{Provider, Quality, UpscalerConfig};
        let cfg = UpscalerConfig {
            provider: Provider::OwnedBilinear,
            quality: Quality::Performance,
        };
        let r = UpscalerRegistry::with_phase6_defaults_from_config(&cfg);
        assert_eq!(r.kinds(), vec![UpscalerKind::OwnedBilinear]);
    }
}
