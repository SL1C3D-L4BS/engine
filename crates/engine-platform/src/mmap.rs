//! Read-only memory-mapped files.
//!
//! POSIX `mmap` is the primitive every later asset path sits on (ADR-029).
//! Loading a pak file via [`MmapRo::open`] gives the kernel a contiguous
//! mapping that the runtime can borrow `&[u8]` slices out of without a
//! single byte of copy. Resident-set size scales with the *working set*
//! the game touches, not with the on-disk pak size — large pak files no
//! longer pay an upfront read-into-Vec cost.
//!
//! `MmapRo` is *not* clonable. The intended sharing model is an external
//! [`Arc<MmapRo>`] — every pak entry references one shared
//! `Arc<MmapRo>` plus a `Range<usize>` into it, so the kernel mapping
//! lasts exactly as long as the last entry that needs it.
//!
//! # Platform support
//!
//! - Linux, macOS — `libc::mmap(MAP_PRIVATE | MAP_POPULATE, PROT_READ)`.
//!   `MAP_POPULATE` pre-faults the mapping so the cold-cache first-touch
//!   cost lands during `open()` and not in the middle of a frame
//!   (Linux only; macOS does not expose the flag).
//! - Windows — [`MmapRo::open`] returns [`io::ErrorKind::Unsupported`].
//!   Windows runtime parity is deferred to Phase 11; until then call
//!   sites can fall back to `Pak::from_bytes(fs::read(path))`.
//!
//! # Safety
//!
//! `MmapRo` keeps the file descriptor alive (an owned [`File`] inside the
//! struct) so the mapping cannot be torn down by a racing close. The
//! `&[u8]` view is sound as long as no one else `ftruncate`s the file
//! shorter than the mapped length while the mapping is live — the asset
//! pipeline writes paks atomically (rename-over) so the live mapping
//! always references an immutable inode, and a writer publishing a new
//! pak rebuilds the mapping on the new inode. ADR-029 spells this out in
//! more detail.

use std::fs::File;
use std::io;
use std::path::Path;
use std::ptr;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::raw::c_void;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod posix {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::raw::c_void;

    pub(super) struct PosixMmap {
        ptr: *const u8,
        len: usize,
        // Kept alive so the mapping cannot outlive the underlying fd.
        _file: File,
    }

    unsafe impl Send for PosixMmap {}
    unsafe impl Sync for PosixMmap {}

    impl PosixMmap {
        pub(super) fn open(path: &Path) -> io::Result<Self> {
            let file = File::open(path)?;
            let len = file.metadata()?.len() as usize;
            if len == 0 {
                // mmap of length 0 is EINVAL; treat as an empty borrow so
                // callers don't have to special-case it.
                return Ok(Self {
                    ptr: std::ptr::NonNull::<u8>::dangling().as_ptr(),
                    len: 0,
                    _file: file,
                });
            }
            let fd = file.as_raw_fd();
            let populate = populate_flag();
            let flags = libc::MAP_PRIVATE | populate;
            // SAFETY: fd is open; addr = NULL lets the kernel pick the
            // mapping. The returned pointer is checked against
            // MAP_FAILED below.
            let raw =
                unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, flags, fd, 0) };
            if raw == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                ptr: raw as *const u8,
                len,
                _file: file,
            })
        }

        pub(super) fn as_slice(&self) -> &[u8] {
            if self.len == 0 {
                return &[];
            }
            // SAFETY: `ptr` came from `mmap` with `len` bytes and is
            // immutably valid for the lifetime of `self` (the mapping
            // drops only when `self` does). PROT_READ guarantees no
            // writes change the bytes underneath us — the only write
            // hazard is `ftruncate` on the underlying file, which the
            // asset pipeline avoids by writing paks atomically.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }

    impl Drop for PosixMmap {
        fn drop(&mut self) {
            if self.len == 0 {
                return;
            }
            // SAFETY: `ptr` was returned by a successful `mmap` with the
            // same length; `munmap` consumes the kernel-side mapping
            // exactly once.
            unsafe {
                libc::munmap(self.ptr as *mut c_void, self.len);
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn populate_flag() -> libc::c_int {
        libc::MAP_POPULATE
    }
    #[cfg(target_os = "macos")]
    fn populate_flag() -> libc::c_int {
        // macOS does not have MAP_POPULATE — accept the cold-first-touch
        // page-fault cost. ADR-029 calls this out.
        0
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod posix {
    use super::*;
    pub(super) struct PosixMmap;
    impl PosixMmap {
        pub(super) fn open(_path: &Path) -> io::Result<Self> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "MmapRo is only implemented for Linux and macOS in Phase 2; \
                 use Pak::from_bytes(fs::read(path)) as a fallback",
            ))
        }
        pub(super) fn as_slice(&self) -> &[u8] {
            &[]
        }
    }
}

/// An immutable, page-aligned view of a file on disk.
///
/// Not [`Clone`]: share by reference via [`Arc<MmapRo>`](std::sync::Arc).
pub struct MmapRo {
    inner: posix::PosixMmap,
}

impl MmapRo {
    /// Maps `path` read-only into the process's address space.
    ///
    /// Returns [`io::ErrorKind::Unsupported`] on platforms outside Linux
    /// and macOS.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            inner: posix::PosixMmap::open(path.as_ref())?,
        })
    }

    /// Returns the mapped bytes as a borrowed slice.
    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_slice()
    }

    /// The length of the mapping in bytes.
    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    /// `true` if the mapping is zero-length (e.g. an empty file).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl std::fmt::Debug for MmapRo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmapRo").field("len", &self.len()).finish()
    }
}

// ---------------------------------------------------------------------------
// MmapAnon — anonymous, page-aligned, RW mapping with an optional PROT_NONE
// guard page (ADR-032). Used by the fiber stack allocator: each per-job
// stack is an MmapAnon with a one-page guard at the low address, so a
// stack overflow segfaults the offending thread instead of trampling
// adjacent stacks.
// ---------------------------------------------------------------------------

/// An anonymous read/write mapping with an optional low-address
/// `PROT_NONE` guard page.
///
/// Allocated via `libc::mmap(MAP_ANON | MAP_PRIVATE)` followed by an
/// `mprotect(PROT_NONE)` of the first page; on drop the *full* region
/// (including the guard) is unmapped. `usable_bytes()` returns the
/// length the caller may write to (the region size minus one page when a
/// guard is requested, the full region otherwise).
///
/// Linux and macOS only. On other targets the constructor returns
/// [`io::ErrorKind::Unsupported`] mirroring the [`MmapRo`] policy.
pub struct MmapAnon {
    ptr: *mut u8,
    region_bytes: usize,
    usable_offset: usize,
    usable_bytes: usize,
}

unsafe impl Send for MmapAnon {}
unsafe impl Sync for MmapAnon {}

impl MmapAnon {
    /// Allocates an anonymous mapping of at least `bytes` usable
    /// read/write bytes, optionally preceded by a `PROT_NONE` guard page.
    ///
    /// Returns [`io::ErrorKind::Unsupported`] on non-POSIX targets.
    pub fn new(bytes: usize, with_guard_page: bool) -> io::Result<Self> {
        anon::open(bytes, with_guard_page)
    }

    /// A raw pointer to the first usable byte of the region.
    pub fn as_ptr(&self) -> *mut u8 {
        // SAFETY: `ptr` is valid for `region_bytes`; the `usable_offset`
        // byte still lies inside the allocation.
        unsafe { self.ptr.add(self.usable_offset) }
    }

    /// The size of the usable (RW) region in bytes.
    pub fn usable_bytes(&self) -> usize {
        self.usable_bytes
    }

    /// The full size of the underlying allocation, including the guard
    /// page when present.
    pub fn region_bytes(&self) -> usize {
        self.region_bytes
    }
}

impl Drop for MmapAnon {
    fn drop(&mut self) {
        if self.region_bytes != 0 && !self.ptr.is_null() {
            anon::close(self.ptr, self.region_bytes);
        }
        self.ptr = ptr::null_mut();
        self.region_bytes = 0;
        self.usable_bytes = 0;
    }
}

impl std::fmt::Debug for MmapAnon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmapAnon")
            .field("usable_bytes", &self.usable_bytes)
            .field("region_bytes", &self.region_bytes)
            .finish()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod anon {
    use super::*;

    pub(super) fn open(bytes: usize, with_guard_page: bool) -> io::Result<MmapAnon> {
        // SAFETY: page-size syscall has no preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let usable_pages = bytes.div_ceil(page).max(1);
        let total_pages = usable_pages + if with_guard_page { 1 } else { 0 };
        let region_bytes = total_pages * page;
        // SAFETY: addr = NULL lets the kernel pick the location; flags and
        // prot are stock anonymous-private RW. The returned pointer is
        // checked against MAP_FAILED.
        let raw = unsafe {
            libc::mmap(
                ptr::null_mut(),
                region_bytes,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        if with_guard_page {
            // SAFETY: `raw` is page-aligned by `mmap`'s contract; `page`
            // is the kernel-reported page size; both are inside the
            // freshly-allocated region.
            let res = unsafe { libc::mprotect(raw, page, libc::PROT_NONE) };
            if res != 0 {
                let err = io::Error::last_os_error();
                // Best-effort cleanup; the mmap leaks at most this region
                // on the error path, which never happens in practice.
                unsafe {
                    libc::munmap(raw, region_bytes);
                }
                return Err(err);
            }
        }
        Ok(MmapAnon {
            ptr: raw as *mut u8,
            region_bytes,
            usable_offset: if with_guard_page { page } else { 0 },
            usable_bytes: usable_pages * page,
        })
    }

    pub(super) fn close(ptr: *mut u8, region_bytes: usize) {
        // SAFETY: `ptr` was returned by a successful `mmap` of length
        // `region_bytes` in `open` and is unmapped at most once (the
        // caller's Drop is the only path to here).
        unsafe {
            libc::munmap(ptr as *mut c_void, region_bytes);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod anon {
    use super::*;

    pub(super) fn open(_bytes: usize, _with_guard_page: bool) -> io::Result<MmapAnon> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MmapAnon is only implemented for Linux and macOS in Phase 3",
        ))
    }

    pub(super) fn close(_ptr: *mut u8, _region_bytes: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Arc;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "engine-platform-mmap-{}-{}.bin",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn open_and_read_round_trips() {
        let path = tmp_path("round-trip");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"hello mmap").unwrap();
            f.sync_all().unwrap();
        }
        let map = Arc::new(MmapRo::open(&path).expect("open mmap"));
        assert_eq!(map.len(), 10);
        assert_eq!(map.as_bytes(), b"hello mmap");
        // Cloning the Arc shares the mapping by reference; both clones
        // point into the same kernel mapping.
        let map2 = Arc::clone(&map);
        assert_eq!(map2.as_bytes(), b"hello mmap");
        drop(map);
        assert_eq!(map2.as_bytes(), b"hello mmap");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn empty_file_maps_as_empty_slice() {
        let path = tmp_path("empty");
        std::fs::File::create(&path).unwrap();
        let map = MmapRo::open(&path).expect("open empty mmap");
        assert!(map.is_empty());
        assert_eq!(map.as_bytes(), b"");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn missing_file_errors() {
        let err = MmapRo::open("/no/such/path/engine-mmap-test").unwrap_err();
        assert!(matches!(
            err.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::Unsupported
        ));
    }
}
