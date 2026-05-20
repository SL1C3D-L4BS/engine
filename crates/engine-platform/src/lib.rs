//! `engine-platform` — operating-system abstraction.
//!
//! Level 0 crate (no engine dependencies). See `ENGINE_SPECIFICATION_v2.0.md`
//! Part IV.1.
//!
//! # Foundation slice
//!
//! This crate currently covers the OS surfaces the foundation layer needs:
//! monotonic time and [frame pacing](time), [filesystem](fs) helpers with
//! atomic writes, filesystem [change watching](watch), [memory
//! mapping](mmap), [host information](sysinfo), and [input event
//! types](input). Windowing and GPU surface creation depend on a
//! compositor and arrive with the renderer in a later phase.

pub mod fs;
pub mod input;
pub mod mmap;
pub mod sysinfo;
pub mod time;
pub mod watch;

pub use input::{ButtonState, InputEvent, Key, Modifiers, MouseButton};
pub use mmap::MmapRo;
pub use sysinfo::{Arch, Os, SystemInfo};
pub use time::FramePacer;
pub use watch::{FileWatcher, PollingWatcher, WatchEvent, WatchKind};

#[cfg(target_os = "linux")]
pub use watch::InotifyWatcher;
