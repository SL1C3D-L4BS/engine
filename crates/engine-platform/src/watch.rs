//! Filesystem change watching.
//!
//! Hot-reload of assets and scripts (spec IV.7, IV.8) needs to observe file
//! changes. Two implementations are provided:
//!
//! - [`PollingWatcher`] — portable, zero-dependency, scans modification times.
//! - [`InotifyWatcher`] — Linux fast path built on `inotify` (no polling).
//!
//! Both satisfy the [`FileWatcher`] trait, which is non-blocking: `poll`
//! returns the changes observed since the previous call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The kind of change observed on a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchKind {
    /// The path appeared.
    Created,
    /// The path's contents changed.
    Modified,
    /// The path disappeared.
    Removed,
}

/// A single observed filesystem change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEvent {
    /// The affected path.
    pub path: PathBuf,
    /// What happened to it.
    pub kind: WatchKind,
}

/// A non-blocking filesystem watcher.
pub trait FileWatcher {
    /// Returns the changes observed since the previous call. Never blocks.
    fn poll(&mut self) -> Vec<WatchEvent>;
}

/// Portable watcher that detects changes by rescanning modification times.
///
/// It walks the watched directory tree on every [`poll`](FileWatcher::poll)
/// and diffs `(mtime, len)` pairs, so it has no operating-system dependency.
pub struct PollingWatcher {
    root: PathBuf,
    snapshot: HashMap<PathBuf, (SystemTime, u64)>,
}

impl PollingWatcher {
    /// Starts watching `root`. The initial directory contents form the
    /// baseline and are not reported as events.
    pub fn new(root: impl AsRef<Path>) -> Self {
        let mut watcher = Self {
            root: root.as_ref().to_path_buf(),
            snapshot: HashMap::new(),
        };
        watcher.snapshot = watcher.scan();
        watcher
    }

    fn scan(&self) -> HashMap<PathBuf, (SystemTime, u64)> {
        let mut out = HashMap::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(path);
                } else if meta.is_file() {
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    out.insert(path, (mtime, meta.len()));
                }
            }
        }
        out
    }
}

impl FileWatcher for PollingWatcher {
    fn poll(&mut self) -> Vec<WatchEvent> {
        let fresh = self.scan();
        let mut events = Vec::new();
        for (path, stamp) in &fresh {
            match self.snapshot.get(path) {
                None => events.push(WatchEvent {
                    path: path.clone(),
                    kind: WatchKind::Created,
                }),
                Some(old) if old != stamp => events.push(WatchEvent {
                    path: path.clone(),
                    kind: WatchKind::Modified,
                }),
                _ => {}
            }
        }
        for path in self.snapshot.keys() {
            if !fresh.contains_key(path) {
                events.push(WatchEvent {
                    path: path.clone(),
                    kind: WatchKind::Removed,
                });
            }
        }
        self.snapshot = fresh;
        events
    }
}

#[cfg(target_os = "linux")]
pub use inotify_impl::InotifyWatcher;

#[cfg(target_os = "linux")]
mod inotify_impl {
    use super::{FileWatcher, WatchEvent, WatchKind};
    use std::collections::HashMap;
    use std::ffi::{CString, OsStr};
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    /// `inotify`-backed watcher (Linux). Registered directories deliver
    /// changes without polling the filesystem.
    pub struct InotifyWatcher {
        fd: libc::c_int,
        watches: HashMap<libc::c_int, PathBuf>,
        buf: Vec<u8>,
    }

    impl InotifyWatcher {
        /// Creates a watcher with no directories registered.
        pub fn new() -> io::Result<Self> {
            // SAFETY: `inotify_init1` takes only flag bits and returns a fresh
            // fd, or -1 with `errno` set.
            let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                fd,
                watches: HashMap::new(),
                buf: vec![0u8; 16 * 1024],
            })
        }

        /// Registers `dir` for create / modify / delete notifications.
        pub fn watch_dir(&mut self, dir: impl AsRef<Path>) -> io::Result<()> {
            let dir = dir.as_ref();
            let c_path = CString::new(dir.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
            let mask = libc::IN_CREATE
                | libc::IN_MODIFY
                | libc::IN_DELETE
                | libc::IN_MOVED_TO
                | libc::IN_MOVED_FROM
                | libc::IN_CLOSE_WRITE;
            // SAFETY: `self.fd` is a live inotify fd; `c_path` is a valid
            // NUL-terminated C string that outlives the call.
            let wd = unsafe { libc::inotify_add_watch(self.fd, c_path.as_ptr(), mask) };
            if wd < 0 {
                return Err(io::Error::last_os_error());
            }
            self.watches.insert(wd, dir.to_path_buf());
            Ok(())
        }
    }

    impl FileWatcher for InotifyWatcher {
        fn poll(&mut self) -> Vec<WatchEvent> {
            const HEADER: usize = 16; // wd(4) + mask(4) + cookie(4) + len(4)
            let mut events = Vec::new();
            loop {
                // SAFETY: `self.fd` is a live fd; the kernel writes at most
                // `buf.len()` bytes into the buffer this call owns.
                let n = unsafe {
                    libc::read(
                        self.fd,
                        self.buf.as_mut_ptr() as *mut libc::c_void,
                        self.buf.len(),
                    )
                };
                if n <= 0 {
                    // `EAGAIN` (no pending events) or an error — stop draining.
                    break;
                }
                let n = n as usize;
                let mut off = 0;
                // The header fields are parsed byte-wise (no pointer cast), so
                // there is no alignment requirement on the buffer.
                while off + HEADER <= n {
                    let wd = i32::from_ne_bytes(self.buf[off..off + 4].try_into().unwrap());
                    let mask = u32::from_ne_bytes(self.buf[off + 4..off + 8].try_into().unwrap());
                    let len = u32::from_ne_bytes(self.buf[off + 12..off + 16].try_into().unwrap())
                        as usize;
                    let name_start = off + HEADER;
                    if name_start + len > n {
                        break;
                    }
                    if let Some(dir) = self.watches.get(&wd) {
                        let raw = &self.buf[name_start..name_start + len];
                        let name = raw.split(|&b| b == 0).next().unwrap_or(&[]);
                        if !name.is_empty()
                            && let Some(kind) = classify(mask)
                        {
                            events.push(WatchEvent {
                                path: dir.join(OsStr::from_bytes(name)),
                                kind,
                            });
                        }
                    }
                    off += HEADER + len;
                }
            }
            events
        }
    }

    fn classify(mask: u32) -> Option<WatchKind> {
        if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 {
            Some(WatchKind::Created)
        } else if mask & (libc::IN_MODIFY | libc::IN_CLOSE_WRITE) != 0 {
            Some(WatchKind::Modified)
        } else if mask & (libc::IN_DELETE | libc::IN_MOVED_FROM) != 0 {
            Some(WatchKind::Removed)
        } else {
            None
        }
    }

    impl Drop for InotifyWatcher {
        fn drop(&mut self) {
            // SAFETY: `self.fd` came from `inotify_init1` and is closed exactly
            // once, here.
            unsafe {
                libc::close(self.fd);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engine-platform-watch-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn polling_watcher_round_trip() {
        let dir = temp_dir("poll");
        let mut watcher = PollingWatcher::new(&dir);
        assert!(watcher.poll().is_empty());

        let file = dir.join("a.txt");
        std::fs::write(&file, b"one").unwrap();
        let created = watcher.poll();
        assert!(
            created
                .iter()
                .any(|e| e.path == file && e.kind == WatchKind::Created)
        );

        std::fs::write(&file, b"a much longer body").unwrap();
        let modified = watcher.poll();
        assert!(
            modified
                .iter()
                .any(|e| e.path == file && e.kind == WatchKind::Modified)
        );

        std::fs::remove_file(&file).unwrap();
        let removed = watcher.poll();
        assert!(
            removed
                .iter()
                .any(|e| e.path == file && e.kind == WatchKind::Removed)
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inotify_watcher_sees_a_new_file() {
        let dir = temp_dir("inotify");
        let mut watcher = InotifyWatcher::new().expect("inotify_init");
        watcher.watch_dir(&dir).expect("watch_dir");

        let file = dir.join("hot.bp");
        std::fs::write(&file, b"reload me").unwrap();

        // inotify delivers asynchronously; poll for up to ~1s.
        let mut seen = false;
        for _ in 0..100 {
            if watcher.poll().iter().any(|e| e.path == file) {
                seen = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(seen, "inotify did not report the new file");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
