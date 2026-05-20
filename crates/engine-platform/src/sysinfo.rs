//! Static host-machine information.
//!
//! This is the foundation slice: enough to report the build target and core
//! count. Device enumeration (GPUs, displays, input devices) arrives with the
//! windowing layer in a later phase.

/// CPU architecture the engine binary was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    /// 64-bit x86.
    X86_64,
    /// 64-bit ARM.
    Aarch64,
    /// 32-bit WebAssembly.
    Wasm32,
    /// Any other architecture.
    Other,
}

/// Operating-system family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux.
    Linux,
    /// Windows.
    Windows,
    /// macOS.
    MacOs,
    /// Any other OS.
    Other,
}

/// A snapshot of host properties relevant to the engine.
#[derive(Clone, Copy, Debug)]
pub struct SystemInfo {
    /// Build-target CPU architecture.
    pub arch: Arch,
    /// Build-target operating system.
    pub os: Os,
    /// Pointer width in bits (32 or 64).
    pub pointer_width_bits: u32,
    /// Number of logical CPUs available to the process.
    pub logical_cores: usize,
}

impl SystemInfo {
    /// Queries the host. The architecture and OS are compile-time constants
    /// (set by `cfg`); the core count is read at runtime.
    pub fn query() -> Self {
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else if cfg!(target_arch = "wasm32") {
            Arch::Wasm32
        } else {
            Arch::Other
        };

        let os = if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Other
        };

        let logical_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        Self {
            arch,
            os,
            pointer_width_bits: usize::BITS,
            logical_cores,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_reports_a_sane_host() {
        let info = SystemInfo::query();
        assert!(info.logical_cores >= 1);
        assert!(info.pointer_width_bits == 32 || info.pointer_width_bits == 64);
        assert_ne!(info.arch, Arch::Other, "CI runs on x86-64 / aarch64");
    }
}
