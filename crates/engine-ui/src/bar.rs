//! `EngineBar` — the editor's fused status / tool bar: the in-engine analogue of
//! the Waybar instrument cluster (a brand-coloured cap followed by segmented
//! readouts). Immediate-mode — build it each frame and submit to the UI draw
//! path. The model and layout are wired here; the GPU draw lands with
//! engine-ui's draw path (ENGINE_SPECIFICATION_v2.0.md Part IV.1).

use crate::theme::{Rgba, geom};

/// A single readout segment: a leading glyph and short text.
pub struct Segment {
    /// Leading Nerd-Font glyph (see [`crate::theme::icon`]).
    pub glyph: &'static str,
    /// Segment text.
    pub text: String,
}

/// A fused instrument-cluster bar: a brand-coloured cap plus ordered segments.
pub struct EngineBar {
    /// Bold cap label (e.g. `"[ENGINE]"`).
    pub cap: &'static str,
    /// Brand colour of the cap (see [`crate::theme::color`]).
    pub brand: Rgba,
    /// Ordered supporting segments, left to right.
    pub segments: Vec<Segment>,
}

impl EngineBar {
    /// Create a bar with a branded cap and no segments.
    #[must_use]
    pub fn new(cap: &'static str, brand: Rgba) -> Self {
        Self {
            cap,
            brand,
            segments: Vec::new(),
        }
    }

    /// Append a segment; chainable.
    #[must_use]
    pub fn segment(mut self, glyph: &'static str, text: impl Into<String>) -> Self {
        self.segments.push(Segment {
            glyph,
            text: text.into(),
        });
        self
    }

    /// Approximate pixel width at a fixed monospace `char_advance`, using the
    /// token geometry (cap padding + per-segment text advance + 1px dividers).
    /// Layout primitive only — no drawing occurs.
    #[must_use]
    pub fn measure(&self, char_advance: u32) -> u32 {
        let cap = geom::PAD_X_LG * 2 + self.cap.chars().count() as u32 * char_advance;
        let segments: u32 = self
            .segments
            .iter()
            .map(|s| {
                let chars = (s.glyph.chars().count() + s.text.chars().count() + 1) as u32;
                geom::PAD_X * 2 + chars * char_advance + 1
            })
            .sum();
        cap + segments
    }
}
