//! GPU buffer wrapper.
//!
//! Owned [`Buffer`] + [`BufferDesc`] + [`BufferUsage`] flags. Maps to
//! `wgpu::Buffer`; no wgpu type surfaces in the public API.

use crate::device::Device;

/// Allowed usages for a [`Buffer`].
///
/// Owned bitflag struct — no `bitflags` crate dependency (workspace policy).
/// Mirrors a deliberately narrow subset of `wgpu::BufferUsages`; only the
/// flags PR 2's smoke + roundtrip tests exercise are exposed. Wider bits
/// land alongside their consumer passes (PR 3+).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BufferUsage(u32);

impl BufferUsage {
    /// Empty set.
    pub const EMPTY: BufferUsage = BufferUsage(0);
    /// Vertex buffer bind point.
    pub const VERTEX: BufferUsage = BufferUsage(1 << 0);
    /// Index buffer bind point.
    pub const INDEX: BufferUsage = BufferUsage(1 << 1);
    /// Uniform buffer binding.
    pub const UNIFORM: BufferUsage = BufferUsage(1 << 2);
    /// Storage buffer binding (SSBO).
    pub const STORAGE: BufferUsage = BufferUsage(1 << 3);
    /// Indirect-draw argument buffer.
    pub const INDIRECT: BufferUsage = BufferUsage(1 << 4);
    /// Source of a buffer-to-buffer / buffer-to-texture copy.
    pub const COPY_SRC: BufferUsage = BufferUsage(1 << 5);
    /// Destination of a buffer-to-buffer / buffer-to-texture copy.
    pub const COPY_DST: BufferUsage = BufferUsage(1 << 6);
    /// CPU read mapping. Used by [`Buffer::read_back`] staging-buffer
    /// readbacks.
    pub const MAP_READ: BufferUsage = BufferUsage(1 << 7);
    /// CPU write mapping.
    pub const MAP_WRITE: BufferUsage = BufferUsage(1 << 8);

    /// Set union.
    pub const fn union(self, other: BufferUsage) -> BufferUsage {
        BufferUsage(self.0 | other.0)
    }

    /// Test for membership.
    pub const fn contains(self, other: BufferUsage) -> bool {
        (self.0 & other.0) == other.0
    }

    fn to_wgpu(self) -> wgpu::BufferUsages {
        let mut u = wgpu::BufferUsages::empty();
        if self.contains(Self::VERTEX) {
            u |= wgpu::BufferUsages::VERTEX;
        }
        if self.contains(Self::INDEX) {
            u |= wgpu::BufferUsages::INDEX;
        }
        if self.contains(Self::UNIFORM) {
            u |= wgpu::BufferUsages::UNIFORM;
        }
        if self.contains(Self::STORAGE) {
            u |= wgpu::BufferUsages::STORAGE;
        }
        if self.contains(Self::INDIRECT) {
            u |= wgpu::BufferUsages::INDIRECT;
        }
        if self.contains(Self::COPY_SRC) {
            u |= wgpu::BufferUsages::COPY_SRC;
        }
        if self.contains(Self::COPY_DST) {
            u |= wgpu::BufferUsages::COPY_DST;
        }
        if self.contains(Self::MAP_READ) {
            u |= wgpu::BufferUsages::MAP_READ;
        }
        if self.contains(Self::MAP_WRITE) {
            u |= wgpu::BufferUsages::MAP_WRITE;
        }
        u
    }
}

impl core::ops::BitOr for BufferUsage {
    type Output = BufferUsage;
    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for BufferUsage {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Buffer descriptor.
#[derive(Clone, Debug)]
pub struct BufferDesc<'a> {
    /// Debug label surfaced in `wgpu::Buffer::label` and oracle reports.
    pub label: &'a str,
    /// Byte size of the buffer. Rounded up to wgpu's required alignment
    /// (4 bytes for most usages) on creation.
    pub size: u64,
    /// Allowed usages.
    pub usage: BufferUsage,
}

/// Owned GPU buffer.
#[derive(Debug)]
pub struct Buffer {
    raw: wgpu::Buffer,
    size: u64,
    usage: BufferUsage,
    device: Device,
}

impl Buffer {
    /// Create a buffer through the device.
    pub fn new(device: &Device, desc: &BufferDesc<'_>) -> Self {
        let raw = device.raw().create_buffer(&wgpu::BufferDescriptor {
            label: Some(desc.label),
            size: desc.size,
            usage: desc.usage.to_wgpu(),
            mapped_at_creation: false,
        });
        Self {
            raw,
            size: desc.size,
            usage: desc.usage,
            device: device.clone(),
        }
    }

    /// Size in bytes (post-alignment).
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Allowed usages this buffer was created with.
    pub fn usage(&self) -> BufferUsage {
        self.usage
    }

    /// Crate-internal access to the underlying `wgpu::Buffer`.
    pub(crate) fn raw(&self) -> &wgpu::Buffer {
        &self.raw
    }

    /// Synchronously read back the buffer's contents.
    ///
    /// Requires [`BufferUsage::MAP_READ`]. The implementation issues a
    /// `device.poll(Wait)` to drive the map-async completion to ready;
    /// only used by tests / tools, not the render loop.
    ///
    /// The completion signal is a [`std::sync::OnceLock`] (single-shot store
    /// the wgpu callback fills before `poll(Wait)` returns) — the
    /// ADR-032 owned-threading guard rejects [`std::sync::Mutex`] /
    /// `std::sync::mpsc` from `engine-gpu`, and `OnceLock` is the
    /// allowed single-init equivalent.
    pub fn read_back(&self) -> Result<Vec<u8>, crate::GpuError> {
        assert!(
            self.usage.contains(BufferUsage::MAP_READ),
            "Buffer::read_back requires BufferUsage::MAP_READ"
        );
        use std::sync::{Arc, OnceLock};
        let slice = self.raw.slice(..);
        let signal: Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>> = Arc::new(OnceLock::new());
        let signal_cb = Arc::clone(&signal);
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = signal_cb.set(res);
        });
        self.device
            .raw()
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| crate::GpuError::BufferMapFailed {
                reason: e.to_string(),
            })?;
        let result = signal
            .get()
            .ok_or_else(|| crate::GpuError::BufferMapFailed {
                reason: "map_async callback did not fire under poll(Wait)".to_string(),
            })?;
        match result {
            Ok(()) => {
                let bytes = slice.get_mapped_range().to_vec();
                self.raw.unmap();
                Ok(bytes)
            }
            Err(e) => Err(crate::GpuError::BufferMapFailed {
                reason: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(BufferUsage::EMPTY, BufferUsage::default());
        assert!(BufferUsage::EMPTY.contains(BufferUsage::EMPTY));
    }

    #[test]
    fn union_is_set_or() {
        let u = BufferUsage::VERTEX | BufferUsage::COPY_DST;
        assert!(u.contains(BufferUsage::VERTEX));
        assert!(u.contains(BufferUsage::COPY_DST));
        assert!(!u.contains(BufferUsage::INDEX));
        assert!(u.contains(BufferUsage::EMPTY));
    }

    #[test]
    fn bitor_assign_matches_bitor() {
        let mut a = BufferUsage::VERTEX;
        a |= BufferUsage::INDEX;
        assert_eq!(a, BufferUsage::VERTEX | BufferUsage::INDEX);
    }

    #[test]
    fn distinct_flags_dont_alias() {
        let all = [
            BufferUsage::VERTEX,
            BufferUsage::INDEX,
            BufferUsage::UNIFORM,
            BufferUsage::STORAGE,
            BufferUsage::INDIRECT,
            BufferUsage::COPY_SRC,
            BufferUsage::COPY_DST,
            BufferUsage::MAP_READ,
            BufferUsage::MAP_WRITE,
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert!(!a.contains(*b), "{a:?} should not contain {b:?}");
                }
            }
        }
    }
}
