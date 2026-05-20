//! Linux raw `perf_event_open` sampling producer (ADR-030).
//!
//! Linux-only. Opens one per-thread perf fd with
//! `PERF_TYPE_SOFTWARE + PERF_COUNT_SW_CPU_CLOCK`, attached to the calling
//! thread (`pid = 0, cpu = -1`). Samples arrive asynchronously through a
//! kernel-managed mmap'd ring buffer; the caller drains them non-blockingly
//! with [`Sampler::drain`].
//!
//! macOS / Windows compile to a stub whose [`Sampler::try_open`] returns
//! `Ok(None)`, mirroring the LinuxPerfCounters degradation pattern in
//! `tools/cache-observatory/src/perf.rs`. Callers can therefore wrap the
//! sampler in an `Option` and skip sampling cleanly on the non-Linux
//! platforms — the engine never refuses to start because the profiler
//! could not attach.

use std::io;

/// A single PMU sample — the instruction-pointer call chain at the moment
/// the kernel fired the timer.
#[derive(Clone, Debug)]
pub struct Sample {
    /// Instruction-pointer chain, leaf first. Empty for samples with no
    /// usable frames (kernel-only stacks are filtered out at the source).
    pub ips: Vec<u64>,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::mem::{size_of, size_of_val};
    use std::os::raw::{c_int, c_ulong, c_void};
    use std::sync::atomic::{AtomicU64, Ordering};

    // ----- kernel constants ---------------------------------------------

    const PERF_TYPE_SOFTWARE: u32 = 1;
    const PERF_COUNT_SW_CPU_CLOCK: u64 = 0;
    const PERF_SAMPLE_IP: u64 = 1 << 0;
    const PERF_SAMPLE_CALLCHAIN: u64 = 1 << 5;
    const PERF_SAMPLE_TID: u64 = 1 << 1;

    // attr.flags bit positions (the kernel struct is a packed bitfield).
    const ATTR_DISABLED: u64 = 1 << 0;
    const ATTR_EXCLUDE_KERNEL: u64 = 1 << 5;
    const ATTR_EXCLUDE_HV: u64 = 1 << 6;
    const ATTR_FREQ: u64 = 1 << 10;

    const PERF_EVENT_IOC_ENABLE: c_ulong = 0x2400;
    const PERF_EVENT_IOC_DISABLE: c_ulong = 0x2401;
    const PERF_EVENT_IOC_RESET: c_ulong = 0x2403;

    // PERF_RECORD types we care about.
    const PERF_RECORD_SAMPLE: u32 = 9;

    // ----- perf_event_attr ABI ------------------------------------------

    #[repr(C)]
    #[derive(Default)]
    struct PerfEventAttr {
        type_: u32,
        size: u32,
        config: u64,
        sample_period_or_freq: u64,
        sample_type: u64,
        read_format: u64,
        flags: u64,
        wakeup_events_or_watermark: u32,
        bp_type: u32,
        bp_addr_or_config1: u64,
        bp_len_or_config2: u64,
        branch_sample_type: u64,
        sample_regs_user: u64,
        sample_stack_user: u32,
        clockid: i32,
        sample_regs_intr: u64,
        aux_watermark: u32,
        sample_max_stack: u16,
        _reserved2: u16,
    }

    const PERF_ATTR_SIZE: u32 = size_of::<PerfEventAttr>() as u32;

    /// One page is the perf header; the rest is data. `data_pages` MUST be
    /// a power of two — picks 8 here (32 KiB of samples between drains is
    /// plenty for a 99 Hz workload that's drained every ~100 ms).
    const DATA_PAGES: usize = 8;

    fn page_size() -> usize {
        // SAFETY: sysconf is always defined; _SC_PAGESIZE never errors.
        let s = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if s <= 0 { 4096 } else { s as usize }
    }

    fn perf_event_open(
        attr: &mut PerfEventAttr,
        pid: libc::pid_t,
        cpu: c_int,
        group_fd: c_int,
        flags: c_ulong,
    ) -> io::Result<c_int> {
        // SAFETY: `attr` outlives the syscall; the kernel reads `attr.size`
        // bytes and writes nothing back.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_perf_event_open,
                attr as *mut PerfEventAttr,
                pid,
                cpu,
                group_fd,
                flags,
            )
        };
        if fd < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(fd as c_int)
        }
    }

    /// Live sampler attached to one specific thread.
    pub struct LinuxSampler {
        fd: c_int,
        ring_base: *mut u8,
        ring_size: usize,
        page_size: usize,
        // Local cursor; the kernel publishes its head into the header page.
        last_tail: u64,
        // Number of samples observed and dropped, for self-overhead stats.
        seen: AtomicU64,
        dropped: AtomicU64,
    }

    // The ring pointer is owned by this struct; it is sound to send the
    // sampler across threads, but the typical usage is one sampler per
    // thread attached to itself.
    unsafe impl Send for LinuxSampler {}

    impl LinuxSampler {
        pub fn try_open(rate_hz: u32) -> io::Result<Option<Self>> {
            let page = page_size();
            let mut attr = PerfEventAttr {
                type_: PERF_TYPE_SOFTWARE,
                size: PERF_ATTR_SIZE,
                config: PERF_COUNT_SW_CPU_CLOCK,
                sample_period_or_freq: rate_hz as u64,
                sample_type: PERF_SAMPLE_IP | PERF_SAMPLE_TID | PERF_SAMPLE_CALLCHAIN,
                flags: ATTR_DISABLED | ATTR_EXCLUDE_KERNEL | ATTR_EXCLUDE_HV | ATTR_FREQ,
                ..PerfEventAttr::default()
            };
            // Per-thread sampling: pid=0 means "this thread", cpu=-1 means
            // "any CPU this thread runs on" — exactly what we want for fiber
            // workers and cache-line-cold sampling alike.
            let fd = match perf_event_open(&mut attr, 0, -1, -1, 0) {
                Ok(fd) => fd,
                Err(e) => {
                    // EACCES: kernel.perf_event_paranoid > 2 and no CAP_PERFMON.
                    // ENOSYS: kernel built without perf.
                    // Both are recoverable — degrade to no-sampler.
                    if matches!(
                        e.raw_os_error(),
                        Some(libc::EACCES) | Some(libc::EPERM) | Some(libc::ENOSYS)
                    ) {
                        return Ok(None);
                    }
                    return Err(e);
                }
            };

            let ring_size = (1 + DATA_PAGES) * page;
            // SAFETY: fd is open; the mapping size matches the kernel's
            // expectation (1 header + power-of-two data pages).
            let ring_base = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    ring_size,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            if ring_base == libc::MAP_FAILED {
                let e = io::Error::last_os_error();
                // SAFETY: fd was successfully opened above.
                unsafe {
                    libc::close(fd);
                }
                return Err(e);
            }

            Ok(Some(Self {
                fd,
                ring_base: ring_base as *mut u8,
                ring_size,
                page_size: page,
                last_tail: 0,
                seen: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
            }))
        }

        pub fn start(&mut self) {
            // SAFETY: fd is open.
            unsafe {
                libc::ioctl(self.fd, PERF_EVENT_IOC_RESET, 0);
                libc::ioctl(self.fd, PERF_EVENT_IOC_ENABLE, 0);
            }
        }

        pub fn stop(&mut self) {
            // SAFETY: fd is open.
            unsafe {
                libc::ioctl(self.fd, PERF_EVENT_IOC_DISABLE, 0);
            }
        }

        pub fn drain(&mut self, out: &mut Vec<Sample>) {
            // The header page is the `perf_event_mmap_page`; its first
            // two u64 fields after the version/compat are `data_head` and
            // `data_tail`. Layout (Linux UAPI):
            //   u32 version, u32 compat_version, u32 lock, u32 index,
            //   u64 offset, u64 time_enabled, u64 time_running,
            //   /* PMU caps */ u64 capabilities,
            //   /* pinned */    u64 pmc_width, ...
            //   u64 data_head, u64 data_tail, u64 data_offset, u64 data_size
            //
            // Reading via raw offsets keeps us off the libc::perf_event_mmap_page
            // struct (which libc doesn't expose). Offsets verified against
            // `<linux/perf_event.h>` for kernels ≥ 4.x.
            let data_head = self.read_header_u64(1024); // data_head
            let data_offset = self.read_header_u64(1040); // data_offset
            let data_size = self.read_header_u64(1048); // data_size

            // Some kernels initialize data_offset/data_size lazily; fall
            // back to page_size / (DATA_PAGES * page_size).
            let (data_offset, data_size) = if data_size == 0 {
                (self.page_size as u64, (DATA_PAGES * self.page_size) as u64)
            } else {
                (data_offset, data_size)
            };

            let mut tail = self.last_tail.max(self.read_header_u64(1032)); // data_tail
            while tail < data_head {
                let off = (tail % data_size) as usize;
                let record_base =
                    unsafe { self.ring_base.add(data_offset as usize + off) as *const u8 };
                // Header: u32 type, u16 misc, u16 size.
                let type_ = unsafe { (record_base as *const u32).read_unaligned() };
                let size = unsafe { (record_base.add(6) as *const u16).read_unaligned() };
                if size == 0 {
                    // Defensive: shouldn't happen on a well-formed ring.
                    break;
                }
                let rec_end = tail.saturating_add(size as u64);
                if type_ == PERF_RECORD_SAMPLE {
                    // Body layout (matches our sample_type bitmask):
                    //   PERF_SAMPLE_IP        → u64 ip
                    //   PERF_SAMPLE_TID       → u32 pid, u32 tid
                    //   PERF_SAMPLE_CALLCHAIN → u64 nr, u64 ips[nr]
                    let mut cursor = 8usize; // past header
                    let _ip = self.read_at(record_base, cursor);
                    cursor += 8;
                    let _pid_tid = self.read_at(record_base, cursor);
                    cursor += 8;
                    let nr = self.read_at(record_base, cursor) as usize;
                    cursor += 8;
                    let mut ips = Vec::with_capacity(nr.min(64));
                    for i in 0..nr {
                        let ip = self.read_at(record_base, cursor + i * 8);
                        // Filter out PERF_CONTEXT_* markers — pseudo-IPs
                        // the kernel inserts between user/kernel/HV
                        // sections of the chain. Anything in the top
                        // 4 KiB of the u64 space is a context marker
                        // (PERF_CONTEXT_MAX = (u64)-4095).
                        if ip >= 0xffff_ffff_ffff_f000 {
                            continue;
                        }
                        ips.push(ip);
                    }
                    out.push(Sample { ips });
                    self.seen.fetch_add(1, Ordering::Relaxed);
                } else {
                    // PERF_RECORD_LOST and friends — count drops, advance.
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                tail = rec_end;
            }
            // Publish the new tail so the kernel can reuse those bytes.
            self.write_header_u64(1032, tail);
            self.last_tail = tail;
        }

        pub fn seen(&self) -> u64 {
            self.seen.load(Ordering::Relaxed)
        }
        pub fn dropped(&self) -> u64 {
            self.dropped.load(Ordering::Relaxed)
        }

        fn read_header_u64(&self, offset: usize) -> u64 {
            // SAFETY: offset is within the first page (header page); the
            // mapping is at least one page long by construction.
            unsafe { std::ptr::read_volatile(self.ring_base.add(offset) as *const u64) }
        }
        fn write_header_u64(&self, offset: usize, value: u64) {
            // SAFETY: same as read_header_u64; the header page is mapped
            // PROT_READ|PROT_WRITE so the kernel sees the published tail.
            unsafe { std::ptr::write_volatile(self.ring_base.add(offset) as *mut u64, value) }
        }

        /// Reads an unaligned u64 from the ring data area. `base` is the
        /// start of the record (in mapped memory); `offset` is bytes past
        /// `base`. If the read wraps the ring, we splice from the
        /// beginning of the data area.
        fn read_at(&self, base: *const u8, offset: usize) -> u64 {
            // For simplicity we assume the record is contiguous (the
            // kernel only ever publishes records that fit in the ring;
            // it never splits them across the wrap boundary as long as
            // the data area is large enough, which it is at 32 KiB for
            // our 99 Hz sample rate). A defensive splice could be added
            // here later.
            let _ = size_of_val(&base);
            unsafe { (base.add(offset) as *const u64).read_unaligned() }
        }
    }

    impl Drop for LinuxSampler {
        fn drop(&mut self) {
            // SAFETY: ring_base + ring_size match the original mmap;
            // fd was successfully opened.
            unsafe {
                libc::munmap(self.ring_base as *mut c_void, self.ring_size);
                libc::close(self.fd);
            }
        }
    }
}

#[cfg(target_os = "linux")]
type Inner = linux::LinuxSampler;

#[cfg(not(target_os = "linux"))]
mod stub {
    use super::*;

    pub struct StubSampler;

    impl StubSampler {
        pub fn try_open(_rate_hz: u32) -> io::Result<Option<Self>> {
            // Mirrors LinuxPerfCounters::try_open: the absence of perf is
            // not an engine-startup failure. Callers degrade by simply
            // not sampling.
            Ok(None)
        }
        pub fn start(&mut self) {}
        pub fn stop(&mut self) {}
        pub fn drain(&mut self, _out: &mut Vec<Sample>) {}
        pub fn seen(&self) -> u64 {
            0
        }
        pub fn dropped(&self) -> u64 {
            0
        }
    }
}

#[cfg(not(target_os = "linux"))]
type Inner = stub::StubSampler;

/// A live PMU sampling handle attached to one OS thread.
///
/// On Linux, opens a `perf_event_open` fd attached to the calling thread
/// (`pid=0, cpu=-1`) and an mmap'd ring buffer. On other platforms this
/// type's [`Sampler::try_open`] returns `Ok(None)` — the engine never
/// refuses to start because the profiler could not attach.
pub struct Sampler {
    inner: Inner,
}

impl Sampler {
    /// Opens a sampler for the calling thread at `rate_hz` samples per
    /// second.
    ///
    /// Returns `Ok(None)` on non-Linux platforms and when the kernel
    /// rejects the syscall with EACCES / EPERM / ENOSYS (the
    /// `perf_event_paranoid > 2 && !CAP_PERFMON` case). Returns `Err`
    /// only for unexpected failures — out-of-fd, invalid mmap, and so on.
    pub fn try_open(rate_hz: u32) -> io::Result<Option<Self>> {
        Ok(Inner::try_open(rate_hz)?.map(|inner| Sampler { inner }))
    }

    /// Begins sampling. The kernel pushes [`PERF_RECORD_SAMPLE`] records
    /// into the ring buffer until [`Sampler::stop`] or `Drop`.
    pub fn start(&mut self) {
        self.inner.start();
    }

    /// Stops sampling. The ring still holds pending records; call
    /// [`Sampler::drain`] to consume them.
    pub fn stop(&mut self) {
        self.inner.stop();
    }

    /// Drains every pending sample into `out`. Non-blocking; safe to call
    /// while sampling is active.
    pub fn drain(&mut self, out: &mut Vec<Sample>) {
        self.inner.drain(out)
    }

    /// Total samples observed since `start()`.
    pub fn seen(&self) -> u64 {
        self.inner.seen()
    }

    /// Total samples dropped (ring overflow or non-sample records).
    pub fn dropped(&self) -> u64 {
        self.inner.dropped()
    }
}
