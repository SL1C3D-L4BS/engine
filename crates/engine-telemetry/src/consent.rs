//! Telemetry consent gate.
//!
//! Telemetry is opt-in (ADR-020): nothing leaves the device until the user
//! has explicitly granted consent, and that grant is recorded on disk so the
//! choice persists across runs. This module owns the gate; the collector
//! consults it before forwarding signals to any off-process sink.
//!
//! The on-disk form is a single deliberately trivial line — `granted = true`
//! or `granted = false` — written to `~/.config/engine/telemetry.toml`. A
//! missing or unreadable file means "not granted": the safe default.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The persisted consent decision plus the file backing it.
#[derive(Clone, Debug)]
pub struct ConsentStore {
    path: PathBuf,
}

/// The default consent file location, `~/.config/engine/telemetry.toml`.
///
/// Returns `None` if the home directory cannot be determined.
pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        Path::new(&home)
            .join(".config")
            .join("engine")
            .join("telemetry.toml"),
    )
}

impl ConsentStore {
    /// Binds a store to an explicit file path (used for the default location
    /// and for tests).
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `true` only if a consent file exists and records an explicit grant.
    ///
    /// Any other state — no file, unreadable file, or an explicit revoke —
    /// reads as not granted.
    pub fn is_granted(&self) -> bool {
        match fs::read_to_string(&self.path) {
            Ok(text) => parse_granted(&text),
            Err(_) => false,
        }
    }

    /// Records an explicit grant, creating the file and its parent directory.
    pub fn grant(&self) -> io::Result<()> {
        self.write(true)
    }

    /// Records an explicit revoke. Telemetry stops at the next gate check.
    pub fn revoke(&self) -> io::Result<()> {
        self.write(false)
    }

    fn write(&self, granted: bool) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.path,
            format!("# [ENGINE] telemetry consent (ADR-020)\ngranted = {granted}\n"),
        )
    }
}

/// Extracts the `granted` boolean from the trivial consent-file format.
fn parse_granted(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "granted")
        .map(|(_, value)| value.trim() == "true")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("engine-consent-{tag}-{}.toml", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn absent_file_means_not_granted() {
        let store = ConsentStore::at(temp_file("absent"));
        assert!(!store.is_granted());
    }

    #[test]
    fn grant_and_revoke_persist() {
        let path = temp_file("toggle");
        let store = ConsentStore::at(&path);

        store.grant().unwrap();
        assert!(store.is_granted());
        // A fresh store reading the same file sees the persisted decision.
        assert!(ConsentStore::at(&path).is_granted());

        store.revoke().unwrap();
        assert!(!store.is_granted());

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn comments_and_whitespace_are_tolerated() {
        assert!(parse_granted("# a comment\n  granted   =   true  \n"));
        assert!(!parse_granted("granted = false"));
        assert!(!parse_granted("nonsense"));
    }
}
