//! Sandboxed `slangc` subprocess wrapper (ADR-019 + ADR-037).
//!
//! The toolchain executes `slangc` as a sandboxed subprocess:
//!
//! - **Absolute path**: the binary is located once via `which slangc`
//!   (or an explicit `SLANGC` env var at construction time), then
//!   invoked with the resolved absolute path. No shell, no $PATH
//!   resolution at compile time.
//! - **Clean environment**: every `slangc` invocation runs with
//!   `Command::env_clear()` so a hostile or unusual user env can't
//!   change the output. The few env vars `slangc` actually needs are
//!   forwarded explicitly (`LANG=C.UTF-8`).
//! - **Detached stdin**: `Stdio::null()` for stdin so a hung pipe
//!   can't stall a build.
//! - **Captured stdout/stderr**: errors surface as
//!   [`SlangcError::Compile`] with the full stderr text, not a
//!   bare exit code.
//!
//! Reproducibility (ADR-038): we record the pinned `slangc` version
//! string ([`SLANGC_PIN`]). The compiler refuses to run if the
//! installed `slangc` reports a different version unless the caller
//! opts in (`Compiler::permissive`). Goldens commit under the
//! pinned version; deviations are caller-visible.

use crate::target::{Stage, Target};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Pinned `slangc` version (ADR-038).
///
/// The compiler asserts the installed binary reports this string.
/// Bumping it is a deliberate act: update this constant, regenerate
/// the reproducibility golden, commit both in the same change.
pub const SLANGC_PIN: &str = "v2026.9";

/// Why a `slangc` invocation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlangcError {
    /// `slangc` was not found on `$PATH` and no `SLANGC` env var was
    /// set. The toolchain is unusable in this build environment.
    NotFound,
    /// `slangc` was found but reports a version different from
    /// [`SLANGC_PIN`]. Override with [`Compiler::permissive`].
    VersionMismatch {
        /// What we expect to find.
        expected: &'static str,
        /// What the installed binary actually reports.
        found: String,
    },
    /// Spawning the subprocess failed (e.g. permission denied,
    /// ENOMEM). The wrapped string is the `std::io::Error::to_string`.
    SpawnFailed(String),
    /// `slangc` ran to completion but reported failure.
    Compile {
        /// Stage (`vertex` / `fragment` / `compute`).
        stage: Stage,
        /// Target backend.
        target: Target,
        /// Exit code, if any.
        exit_code: Option<i32>,
        /// Full captured stderr.
        stderr: String,
    },
    /// `slangc` emitted no output bytes for an apparently-successful
    /// compilation. The artefact would be empty; treat as a hard
    /// failure.
    EmptyOutput {
        /// Target whose output was empty.
        target: Target,
    },
    /// An on-disk I/O error reading back the output file `slangc`
    /// wrote.
    ReadOutputFailed(String),
}

impl std::fmt::Display for SlangcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "slangc not found on $PATH; set SLANGC env var"),
            Self::VersionMismatch { expected, found } => write!(
                f,
                "slangc version mismatch: expected {expected}, found {found}"
            ),
            Self::SpawnFailed(why) => write!(f, "spawning slangc failed: {why}"),
            Self::Compile {
                stage,
                target,
                exit_code,
                stderr,
            } => write!(
                f,
                "slangc compile failed (stage={stage:?}, target={target:?}, exit={exit_code:?}):\n{stderr}"
            ),
            Self::EmptyOutput { target } => {
                write!(f, "slangc emitted no bytes for target {target:?}")
            }
            Self::ReadOutputFailed(why) => write!(f, "read slangc output failed: {why}"),
        }
    }
}

impl std::error::Error for SlangcError {}

/// Locates `slangc` and pins its version.
///
/// `slangc -v` prints `vYYYY.N` to **stderr** (verified against the
/// installed v2026.9). Use [`Compiler::permissive`] to disable the
/// pin check; the golden oracle still records what was used.
#[derive(Clone, Debug)]
pub struct Compiler {
    path: PathBuf,
    version: String,
    pinned: bool,
}

impl Compiler {
    /// Locates `slangc` and checks its reported version against
    /// [`SLANGC_PIN`].
    pub fn locate() -> Result<Self, SlangcError> {
        let path = resolve_path()?;
        let version = probe_version(&path)?;
        if version.trim() != SLANGC_PIN {
            return Err(SlangcError::VersionMismatch {
                expected: SLANGC_PIN,
                found: version,
            });
        }
        Ok(Self {
            path,
            version,
            pinned: true,
        })
    }

    /// Locates `slangc` without enforcing the pinned version. The
    /// reproducibility oracle uses this to surface "compiled
    /// successfully but goldens won't match" as a diagnostic rather
    /// than a hard failure.
    pub fn permissive() -> Result<Self, SlangcError> {
        let path = resolve_path()?;
        let version = probe_version(&path)?;
        Ok(Self {
            path,
            version,
            pinned: false,
        })
    }

    /// Absolute path of the resolved binary.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What `slangc -v` reported.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// True iff the version pin is in effect.
    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Runs `slangc` against `source` for one (stage, target) pair.
    /// Returns the captured output bytes; the caller wraps them with
    /// reflection JSON to produce a full [`crate::Artifact`].
    ///
    /// Output is written to a temp file under the system tmpdir and
    /// read back into memory — this is the only way `slangc` emits
    /// binary targets (it refuses to write SPIR-V to a pipe).
    pub fn compile(
        &self,
        source: &Path,
        entry: &str,
        stage: Stage,
        target: Target,
    ) -> Result<Vec<u8>, SlangcError> {
        self.compile_with_reflection(source, entry, stage, target, None)
            .map(|(bytes, _)| bytes)
    }

    /// Runs `slangc` and, if `reflection_out` is `Some`, also emits a
    /// `.refl.json` reflection side-table to that path. Returns
    /// `(bytes, reflection_json)` — the reflection is read back from
    /// disk and discarded if `reflection_out` was `None`.
    pub fn compile_with_reflection(
        &self,
        source: &Path,
        entry: &str,
        stage: Stage,
        target: Target,
        reflection_out: Option<&Path>,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), SlangcError> {
        let pid = std::process::id();
        let nonce = monotonic_nonce();
        let tmp_dir = std::env::temp_dir();
        let stem = format!("engine-shader-{pid}-{nonce}");
        let out_path = tmp_dir.join(format!("{stem}.{}", target.extension()));
        let refl_path = reflection_out
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| tmp_dir.join(format!("{stem}.refl.json")));

        // `-o <out>` plus `-reflection-json <refl>`. The remaining
        // flags route the entry point and target.
        let mut cmd = Command::new(&self.path);
        cmd.env_clear()
            .env("LANG", "C.UTF-8")
            .arg("-target")
            .arg(target.slangc_flag())
            .arg("-stage")
            .arg(stage.slangc_flag())
            .arg("-entry")
            .arg(entry)
            .arg("-o")
            .arg(&out_path)
            .arg("-reflection-json")
            .arg(&refl_path)
            .arg(source)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .map_err(|e| SlangcError::SpawnFailed(e.to_string()))?;

        if !output.status.success() {
            // Clean up any partial files before returning.
            let _ = std::fs::remove_file(&out_path);
            if reflection_out.is_none() {
                let _ = std::fs::remove_file(&refl_path);
            }
            return Err(SlangcError::Compile {
                stage,
                target,
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let bytes =
            std::fs::read(&out_path).map_err(|e| SlangcError::ReadOutputFailed(e.to_string()))?;
        let _ = std::fs::remove_file(&out_path);

        if bytes.is_empty() {
            if reflection_out.is_none() {
                let _ = std::fs::remove_file(&refl_path);
            }
            return Err(SlangcError::EmptyOutput { target });
        }

        let refl_bytes = match reflection_out {
            Some(_) => None,
            None => match std::fs::read(&refl_path) {
                Ok(b) => {
                    let _ = std::fs::remove_file(&refl_path);
                    Some(b)
                }
                Err(_) => Some(Vec::new()),
            },
        };
        Ok((bytes, refl_bytes))
    }
}

fn resolve_path() -> Result<PathBuf, SlangcError> {
    if let Ok(explicit) = std::env::var("SLANGC")
        && !explicit.is_empty()
    {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Ok(p);
        }
    }
    // Hand-rolled $PATH walk — avoids depending on `which`.
    if let Some(path_env) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_env) {
            let candidate = dir.join("slangc");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(SlangcError::NotFound)
}

fn probe_version(path: &Path) -> Result<String, SlangcError> {
    let output = Command::new(path)
        .env_clear()
        .env("LANG", "C.UTF-8")
        .arg("-v")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| SlangcError::SpawnFailed(e.to_string()))?;
    // `slangc -v` prints to stderr.
    let text = if !output.stderr.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    // Extract the first `v<YYYY>.<N>` token.
    for word in text.split_whitespace() {
        if word.starts_with('v')
            && word.len() >= 4
            && word[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            return Ok(word.trim().to_string());
        }
    }
    Ok(text.trim().to_string())
}

/// Monotonic per-process counter the temp-file naming uses to avoid
/// collisions between concurrent compile calls.
fn monotonic_nonce() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}
