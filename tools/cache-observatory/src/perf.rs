//! Owned `perf_event_open` plumbing.
//!
//! Linux-only. Opens a counter group on the current process, reads cycles,
//! retired instructions, L1d-read-misses, and LLC-read-misses atomically via
//! `PERF_FORMAT_GROUP`. No third-party perf crate (R-02); the syscall is
//! invoked through [`libc::syscall`] and the `perf_event_attr` ABI is
//! declared inline.
//!
//! Falls back gracefully: if `perf_event_open` fails with `EACCES` (the
//! common case when `/proc/sys/kernel/perf_event_paranoid > 2` and the
//! process lacks `CAP_PERFMON`) the caller is expected to drop the request
//! and run wall-clock only.

#![cfg(target_os = "linux")]

use std::io;
use std::mem::{size_of, size_of_val};
use std::os::raw::{c_int, c_ulong};

use crate::timer::PerfSample;

// --- kernel constants -------------------------------------------------

const PERF_TYPE_HARDWARE: u32 = 0;
const PERF_TYPE_HW_CACHE: u32 = 3;

const PERF_COUNT_HW_CPU_CYCLES: u64 = 0;
const PERF_COUNT_HW_INSTRUCTIONS: u64 = 1;

// Cache-event config = cache_id | (op_id << 8) | (result_id << 16).
const PERF_COUNT_HW_CACHE_L1D: u64 = 0;
const PERF_COUNT_HW_CACHE_LL: u64 = 2;
const PERF_COUNT_HW_CACHE_OP_READ: u64 = 0;
const PERF_COUNT_HW_CACHE_RESULT_MISS: u64 = 1;

fn cache_config(cache: u64, op: u64, result: u64) -> u64 {
    cache | (op << 8) | (result << 16)
}

// PERF_FORMAT_GROUP makes one `read()` on the leader fd return every counter
// in the group, atomically.
const PERF_FORMAT_GROUP: u64 = 1 << 3;

// `attr.flags` bit positions (it is a packed bitfield in <linux/perf_event.h>).
const ATTR_FLAG_DISABLED: u64 = 1 << 0;
const ATTR_FLAG_EXCLUDE_KERNEL: u64 = 1 << 5;
const ATTR_FLAG_EXCLUDE_HV: u64 = 1 << 6;

// ioctl request numbers. _IO('$', N) on every Linux arch is `(0x24 << 8) | N`.
const PERF_EVENT_IOC_ENABLE: c_ulong = 0x2400;
const PERF_EVENT_IOC_DISABLE: c_ulong = 0x2401;
const PERF_EVENT_IOC_RESET: c_ulong = 0x2403;
const PERF_IOC_FLAG_GROUP: c_ulong = 1;

// --- perf_event_attr ABI ---------------------------------------------

// 128 bytes spans the kernel struct up to PERF_ATTR_SIZE_VER6 (Linux 5.x+).
// Only the first few fields are named; the trailing buffer is zero-init and
// keeps us forward-compatible with newer kernel extensions.
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
    _trailing: [u64; 11],
}

const PERF_ATTR_SIZE: u32 = size_of::<PerfEventAttr>() as u32;

fn build_attr(type_: u32, config: u64) -> PerfEventAttr {
    PerfEventAttr {
        type_,
        size: PERF_ATTR_SIZE,
        config,
        read_format: PERF_FORMAT_GROUP,
        flags: ATTR_FLAG_DISABLED | ATTR_FLAG_EXCLUDE_KERNEL | ATTR_FLAG_EXCLUDE_HV,
        ..PerfEventAttr::default()
    }
}

// Direct syscall wrapper. `pid = 0` is "this process", `cpu = -1` is "any
// CPU", `group_fd = -1` opens a new group (leader); subsequent calls pass
// the leader's fd to chain followers.
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

// --- public type ------------------------------------------------------

/// A live perf-event counter group on the current process.
pub struct LinuxPerfCounters {
    leader: c_int,
    followers: Vec<c_int>,
    // Slot indices into `followers[..]` for each counter — `None` if that
    // specific event was rejected by the kernel (e.g. unsupported hardware
    // event) but the group is otherwise usable.
    slot_cycles: Option<usize>,
    slot_instructions: Option<usize>,
    slot_l1d: Option<usize>,
    slot_llc: Option<usize>,
}

impl LinuxPerfCounters {
    /// Tries to open the counter group. Returns `Ok(None)` if every event
    /// the kernel offered was unavailable; `Err` only when the *first*
    /// (cycles) event fails — that is the signal that perf-event is
    /// completely off-limits on this host.
    pub fn try_open() -> io::Result<Option<Self>> {
        // Leader: CPU cycles. Always available on every CPU perf supports.
        let mut attr = build_attr(PERF_TYPE_HARDWARE, PERF_COUNT_HW_CPU_CYCLES);
        let leader = perf_event_open(&mut attr, 0, -1, -1, 0)?;

        let mut followers = Vec::new();
        let slot_cycles = Some(usize::MAX); // leader's value is index 0 of the read
        let _ = slot_cycles;

        let mut slot_instructions = None;
        if let Ok(fd) = open_follower(leader, PERF_TYPE_HARDWARE, PERF_COUNT_HW_INSTRUCTIONS) {
            slot_instructions = Some(followers.len());
            followers.push(fd);
        }
        let mut slot_l1d = None;
        if let Ok(fd) = open_follower(
            leader,
            PERF_TYPE_HW_CACHE,
            cache_config(
                PERF_COUNT_HW_CACHE_L1D,
                PERF_COUNT_HW_CACHE_OP_READ,
                PERF_COUNT_HW_CACHE_RESULT_MISS,
            ),
        ) {
            slot_l1d = Some(followers.len());
            followers.push(fd);
        }
        let mut slot_llc = None;
        if let Ok(fd) = open_follower(
            leader,
            PERF_TYPE_HW_CACHE,
            cache_config(
                PERF_COUNT_HW_CACHE_LL,
                PERF_COUNT_HW_CACHE_OP_READ,
                PERF_COUNT_HW_CACHE_RESULT_MISS,
            ),
        ) {
            slot_llc = Some(followers.len());
            followers.push(fd);
        }

        Ok(Some(Self {
            leader,
            followers,
            slot_cycles: Some(0), // leader is index 0
            slot_instructions: slot_instructions.map(|i| i + 1),
            slot_l1d: slot_l1d.map(|i| i + 1),
            slot_llc: slot_llc.map(|i| i + 1),
        }))
    }

    /// Resets every counter to zero and starts counting.
    pub fn start(&mut self) {
        unsafe {
            libc::ioctl(self.leader, PERF_EVENT_IOC_RESET, PERF_IOC_FLAG_GROUP);
            libc::ioctl(self.leader, PERF_EVENT_IOC_ENABLE, PERF_IOC_FLAG_GROUP);
        }
    }

    /// Stops the counter group and returns a [`PerfSample`] of the values
    /// observed since the matching [`start`](Self::start) call.
    pub fn snapshot(&mut self) -> PerfSample {
        unsafe {
            libc::ioctl(self.leader, PERF_EVENT_IOC_DISABLE, PERF_IOC_FLAG_GROUP);
        }
        // PERF_FORMAT_GROUP read layout: u64 nr, then nr * u64 values.
        let n_events = 1 + self.followers.len();
        let mut buf: Vec<u64> = vec![0; 1 + n_events];
        let n_bytes = size_of_val(&buf[..]);
        let ret = unsafe { libc::read(self.leader, buf.as_mut_ptr() as *mut _, n_bytes) };
        let mut out = PerfSample::default();
        if ret <= 0 {
            return out;
        }
        let nr = buf[0] as usize;
        if nr != n_events {
            // Layout drift; drop the sample rather than mis-attribute.
            return out;
        }
        // Indices: buf[0] = nr; buf[1..] = values, in registration order
        // (leader first).
        let value = |slot: Option<usize>| slot.and_then(|s| buf.get(1 + s).copied());
        out.cycles = value(self.slot_cycles);
        out.instructions = value(self.slot_instructions);
        out.l1d_misses = value(self.slot_l1d);
        out.llc_misses = value(self.slot_llc);
        out
    }
}

impl Drop for LinuxPerfCounters {
    fn drop(&mut self) {
        unsafe {
            for &fd in &self.followers {
                libc::close(fd);
            }
            libc::close(self.leader);
        }
    }
}

fn open_follower(group_leader: c_int, type_: u32, config: u64) -> io::Result<c_int> {
    let mut attr = build_attr(type_, config);
    perf_event_open(&mut attr, 0, -1, group_leader, 0)
}
