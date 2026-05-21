//! Source-file extension routing.
//!
//! The sli language is the canonical name; `.sli` is its canonical file
//! extension. `.bp` is kept as a legacy alias because pre-v1.0 game projects
//! and asset paks already shipped scripts with that extension (Phase 0
//! contract-exempt). Both forms are accepted on input; the canonical form
//! is preferred on output (asset packing, hot-reload watch lists).

/// The on-disk role of a script file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    /// Canonical `.sli` source.
    SliCanonical,
    /// Legacy `.bp` alias kept for one major version.
    SliBpAlias,
}

impl SourceKind {
    /// The file extension this kind matches, without the leading dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::SliCanonical => "sli",
            Self::SliBpAlias => "bp",
        }
    }
}

/// Classifies `path` by its file extension. Case-insensitive on the
/// extension; the rest of the path is opaque.
pub fn classify(path: &str) -> Option<SourceKind> {
    let dot = path.rfind('.')?;
    let ext = &path[dot + 1..];
    match ext.to_ascii_lowercase().as_str() {
        "sli" => Some(SourceKind::SliCanonical),
        "bp" => Some(SourceKind::SliBpAlias),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_extensions() {
        assert_eq!(classify("game/main.sli"), Some(SourceKind::SliCanonical));
        assert_eq!(classify("game/main.SLI"), Some(SourceKind::SliCanonical));
        assert_eq!(classify("game/legacy.bp"), Some(SourceKind::SliBpAlias));
        assert_eq!(classify("game/legacy.BP"), Some(SourceKind::SliBpAlias));
        assert_eq!(classify("README.md"), None);
        assert_eq!(classify("no_extension"), None);
    }
}
