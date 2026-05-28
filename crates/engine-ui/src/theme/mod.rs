//! Design theme for the engine UI — colours, glyphs, geometry, and motion
//! generated from the workstation design tokens (`~/.dotfiles/system/tokens.toml`)
//! so the engine's in-app chrome stays in lockstep with Waybar / Niri / Neovim.
//!
//! Regenerate [`palette`] with `just gen-palette` after editing `tokens.toml`.

pub mod palette;

pub use palette::{Rgba, color, geom, icon, motion};
