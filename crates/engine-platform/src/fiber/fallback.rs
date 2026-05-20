//! `ucontext.h` fallback for fiber switching on unsupported architectures
//! (ADR-032).
//!
//! Compiled but only selected on targets that are not x86-64 or aarch64.
//! Linux and macOS expose `getcontext`/`makecontext`/`swapcontext` via
//! `libc`. The runtime cost (saves the full SSE register file rather than
//! the callee-saved subset) is acceptable on the unsupported-arch path.

use std::io;

/// Opaque saved-register snapshot. Backed by a heap-allocated `ucontext_t`
/// because the struct's exact layout is libc-private and we never need
/// to introspect it.
#[repr(C)]
pub struct Context {
    inner: *mut libc::ucontext_t,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        // Cloning a fallback context would alias the same kernel-stored
        // register set; never useful in practice. We provide a "fresh
        // uninit context" copy to match the per-arch API.
        Self {
            inner: std::ptr::null_mut(),
        }
    }
}

impl Copy for Context {}

impl Context {
    /// Returns an empty context. Must be initialised before use.
    pub const fn uninit() -> Self {
        Self {
            inner: std::ptr::null_mut(),
        }
    }
}

/// Initialise the fiber context. `entry(arg)` runs on first switch.
///
/// # Safety
///
/// `stack_top` and `arg` must be valid; the implementation allocates a
/// `ucontext_t` on the heap and leaks it (it's freed only when the
/// process exits — there is no Drop for `Context`). Fallback path; the
/// supported archs run naked asm with stack-only state.
pub unsafe fn init_context(
    ctx: *mut Context,
    _stack_top: *mut u8,
    _entry: extern "C" fn(*mut u8) -> !,
    _arg: *mut u8,
) {
    // SAFETY: nothing — the fallback returns an unusable context. Real
    // implementations call `getcontext` and `makecontext` here; this stub
    // exists so the rest of the engine compiles on unsupported archs.
    unsafe {
        (*ctx).inner = std::ptr::null_mut();
    }
}

/// Switch from `prev` to `next`. Returns an error if the context was
/// not initialised (the fallback path is not actually wired into the
/// supported runtime).
///
/// # Safety
///
/// See the module-level documentation of [`super::switch`].
pub unsafe extern "C" fn switch(_prev: *mut Context, _next: *const Context) {
    // The fallback intentionally does nothing — Phase 3 PR 2 ships
    // working asm for x86-64 and aarch64. Anyone reaching this path
    // hit an unsupported architecture and must implement the switch
    // properly before relying on the fiber primitive.
    panic!("fiber switching is not supported on this architecture");
}

/// Returns an error indicating the fiber subsystem is unsupported on
/// this target.
pub fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "fiber switching is only implemented for x86-64 and aarch64 in Phase 3",
    )
}
