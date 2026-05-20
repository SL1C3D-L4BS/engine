//! Structured logging — JSON-lines output with size-based rotation.
//!
//! Every log entry is one self-contained JSON object on its own line (the
//! JSON-lines convention), so logs are greppable as text and parseable as
//! data without a separate schema file (spec X.6). The writer rotates the
//! file once it passes a size threshold so a long-running session cannot fill
//! the disk.
//!
//! `Trace` and `Debug` records are dropped in release builds — verbose
//! logging is a development aid and must not cost anything in a shipped game.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Severity of a log record, least to most severe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Fine-grained tracing. Stripped in release.
    Trace,
    /// Debugging detail. Stripped in release.
    Debug,
    /// Normal operational information.
    Info,
    /// A recoverable problem.
    Warn,
    /// A failure.
    Error,
}

impl LogLevel {
    /// The lowercase wire name of the level.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Whether records at this level are dropped in release builds.
    pub fn stripped_in_release(self) -> bool {
        matches!(self, Self::Trace | Self::Debug)
    }
}

/// One structured log record (spec X.6 schema).
#[derive(Clone, Debug, PartialEq)]
pub struct LogRecord {
    /// Monotonic timestamp, nanoseconds since process start.
    pub timestamp_ns: u64,
    /// Severity.
    pub level: LogLevel,
    /// Originating subsystem, lowercase (e.g. `"render"`).
    pub subsystem: String,
    /// A short stable event identifier (e.g. `"asset.reload"`).
    pub target: String,
    /// Human-readable message.
    pub message: String,
    /// Flat structured key/value context.
    pub fields: Vec<(String, String)>,
}

impl LogRecord {
    /// Renders the record as a single JSON-lines entry (no trailing newline).
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(128);
        s.push('{');
        s.push_str(&format!("\"ts\":{}", self.timestamp_ns));
        s.push_str(&format!(",\"level\":\"{}\"", self.level.as_str()));
        s.push_str(",\"subsystem\":");
        push_json_str(&mut s, &self.subsystem);
        s.push_str(",\"target\":");
        push_json_str(&mut s, &self.target);
        s.push_str(",\"msg\":");
        push_json_str(&mut s, &self.message);
        s.push_str(",\"fields\":{");
        for (i, (key, value)) in self.fields.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            push_json_str(&mut s, key);
            s.push(':');
            push_json_str(&mut s, value);
        }
        s.push_str("}}");
        s
    }
}

/// Appends a JSON string literal (with escaping) to `out`.
fn push_json_str(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The default rotation threshold: 50 MiB (spec X.6).
pub const DEFAULT_ROTATION_BYTES: u64 = 50 * 1024 * 1024;

/// A JSON-lines log file that rotates when it grows past a byte threshold.
pub struct LogWriter {
    path: PathBuf,
    file: File,
    written: u64,
    rotation_bytes: u64,
    rotations: u32,
}

impl LogWriter {
    /// Opens (creating or truncating) a log file at `path`, rotating once it
    /// exceeds `rotation_bytes`.
    pub fn create(path: impl Into<PathBuf>, rotation_bytes: u64) -> io::Result<Self> {
        let path = path.into();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)?;
        Ok(Self {
            path,
            file,
            written: 0,
            rotation_bytes,
            rotations: 0,
        })
    }

    /// Writes one record. Records stripped in release builds are silently
    /// dropped there; in debug builds every level is written.
    pub fn write(&mut self, record: &LogRecord) -> io::Result<()> {
        if cfg!(not(debug_assertions)) && record.level.stripped_in_release() {
            return Ok(());
        }
        let mut line = record.to_json();
        line.push('\n');
        let bytes = line.as_bytes();
        if self.written + bytes.len() as u64 > self.rotation_bytes {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    /// Number of times the log has rotated.
    pub fn rotations(&self) -> u32 {
        self.rotations
    }

    /// Renames the live file to a numbered archive and starts a fresh one.
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.rotations += 1;
        let archive = archive_path(&self.path, self.rotations);
        fs::rename(&self.path, &archive)?;
        self.file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

/// Builds the archive name for the `n`th rotation, e.g. `engine.log` →
/// `engine.1.log`.
fn archive_path(path: &Path, n: u32) -> PathBuf {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("log");
    let ext = path.extension().and_then(|s| s.to_str());
    let name = match ext {
        Some(ext) => format!("{stem}.{n}.{ext}"),
        None => format!("{stem}.{n}"),
    };
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("engine-log-{tag}-{}.log", std::process::id()));
        p
    }

    fn record(level: LogLevel, msg: &str) -> LogRecord {
        LogRecord {
            timestamp_ns: 1_234,
            level,
            subsystem: "render".into(),
            target: "frame.present".into(),
            message: msg.into(),
            fields: vec![("frame".into(), "9001".into())],
        }
    }

    #[test]
    fn json_line_matches_the_schema() {
        let json = record(LogLevel::Info, "presented").to_json();
        assert!(json.starts_with('{') && json.ends_with('}'));
        assert!(!json.contains('\n'));
        for key in [
            "\"ts\":",
            "\"level\":",
            "\"subsystem\":",
            "\"target\":",
            "\"msg\":",
            "\"fields\":",
        ] {
            assert!(json.contains(key), "missing {key} in {json}");
        }
        assert!(json.contains("\"level\":\"info\""));
        assert!(json.contains("\"frame\":\"9001\""));
    }

    #[test]
    fn special_characters_are_escaped() {
        let mut rec = record(LogLevel::Warn, "line one\nline \"two\"\tend");
        rec.fields.clear();
        let json = rec.to_json();
        assert!(json.contains("line one\\nline \\\"two\\\"\\tend"));
        assert!(!json.contains('\n')); // the literal newline did not leak
    }

    #[test]
    fn writer_rotates_past_the_threshold() {
        let path = temp_path("rotate");
        let _ = fs::remove_file(&path);
        // A threshold small enough that a few records trigger rotation.
        let mut writer = LogWriter::create(&path, 200).unwrap();
        for _ in 0..20 {
            writer
                .write(&record(LogLevel::Info, "a reasonably long message"))
                .unwrap();
        }
        assert!(writer.rotations() >= 1);
        assert!(archive_path(&path, 1).exists());

        // Clean up archives and the live file.
        for n in 0..=writer.rotations() {
            let _ = fs::remove_file(if n == 0 {
                path.clone()
            } else {
                archive_path(&path, n)
            });
        }
    }

    #[test]
    fn release_strips_verbose_levels() {
        assert!(LogLevel::Trace.stripped_in_release());
        assert!(LogLevel::Debug.stripped_in_release());
        assert!(!LogLevel::Info.stripped_in_release());
        assert!(!LogLevel::Error.stripped_in_release());
    }
}
