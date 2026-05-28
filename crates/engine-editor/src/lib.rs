//! `engine-editor` — the editor application and panels
//!
//! Level 3 crate. See ENGINE_SPECIFICATION_v2.0.md Part IV.1.

use engine_ui::bar::EngineBar;
use engine_ui::theme::{color, icon};

/// The editor's top status bar — the engine's fused instrument cluster, themed
/// from the workstation design tokens via `engine-ui`. Rebuilt each frame; the
/// GPU draw lands with engine-ui's draw path (Part IV.1).
#[must_use]
pub fn status_bar() -> EngineBar {
    EngineBar::new("[ENGINE]", color::PRIMARY).segment(icon::ENGINE, "editor")
}
