//! User-space cooperative fibers (ADR-032).
//!
//! A fiber is a stack-bound, cooperatively-scheduled coroutine: it owns its
//! own stack (a [`MmapAnon`](super::mmap::MmapAnon) with a `PROT_NONE` guard
//! page, ADR-029/ADR-032) and a saved register snapshot. Switching is
//! callee-saved-registers-only — naked asm on x86-64 + aarch64, falling
//! back to `ucontext.h` on anything else.
//!
//! Phase 3 PR 2 ships fibers as an exported primitive but does *not* wire
//! them into the job scheduler — the static R/W-DAG scheduler ADR-032
//! describes is run-to-completion per-job, which is sufficient for
//! R/W-disjoint commutative graphs (the property the oracle proves). The
//! fiber primitive is here for Phase 4+ use cases that genuinely need
//! cooperative yield (long-running asset jobs, frame-spanning work).

use crate::mmap::MmapAnon;
use std::io;
use std::mem::ManuallyDrop;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod fallback;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
use aarch64 as imp;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
use fallback as imp;
#[cfg(target_arch = "x86_64")]
use x86_64 as imp;

/// Default stack size for a fiber: 64 KiB usable + 1 page `PROT_NONE`
/// guard. 64 KiB is the standard small-stack value in cooperative
/// scheduler libraries (Boost.Context, golang's stack growth start, etc.)
/// and fits two fibers per 128 KiB superpage on Linux/aarch64.
pub const DEFAULT_FIBER_STACK_BYTES: usize = 64 * 1024;

/// Saved register context for one fiber. Layout is architecture-defined
/// (see [`imp::Context`]).
#[doc(hidden)]
pub use imp::Context;

/// Switch from the currently-running fiber to `next`, saving the current
/// register set into `prev`.
///
/// # Safety
///
/// - `prev` must be a writable, properly-aligned [`Context`] pointer; the
///   caller must own exclusive access for the duration of the switch.
/// - `next` must be a previously-initialised [`Context`] (via
///   [`Fiber::new`] or the implicit "scheduler" context that
///   [`switch_init`] sets up).
/// - The stack and `Context` of the *current* fiber must remain valid
///   while another fiber holds the CPU — typically by living on the
///   thread's main stack or inside a `Box`/`ManuallyDrop` owned by a
///   scheduler.
pub unsafe fn switch(prev: *mut Context, next: *const Context) {
    // SAFETY: forwarded from the caller; the per-arch impl performs the
    // register save/restore inside a naked asm block.
    unsafe {
        imp::switch(prev, next);
    }
}

/// A live fiber. Owns its stack via [`MmapAnon`] and its saved register
/// [`Context`]; both drop together. The fiber is *not* runnable until its
/// `Context` has been initialised by [`Fiber::new`].
pub struct Fiber {
    // Order matters: `context` must be dropped *before* the stack it
    // points into.
    context: ManuallyDrop<Context>,
    stack: ManuallyDrop<MmapAnon>,
    // Type-erased boxed closure the entry trampoline owns and calls.
    // Lives until the fiber's first switch returns to the trampoline's
    // post-call cleanup. Stored here so it's not dropped while the
    // trampoline is running.
    closure: Option<Box<FiberEntry>>,
}

type FiberEntry = dyn FnOnce() + Send;

impl Fiber {
    /// Allocates a fiber with the default stack size and an entry
    /// closure. The closure runs the first time the fiber is switched to;
    /// when it returns, the fiber's stack is exhausted and switching to
    /// it again is undefined behaviour (the per-arch trampolines do
    /// *not* implement re-entry — that's a Phase 4 concern).
    pub fn new<F>(f: F) -> io::Result<Self>
    where
        F: FnOnce() + Send + 'static,
    {
        Self::with_stack_size(DEFAULT_FIBER_STACK_BYTES, f)
    }

    /// [`Fiber::new`] with an explicit usable stack size.
    pub fn with_stack_size<F>(stack_bytes: usize, f: F) -> io::Result<Self>
    where
        F: FnOnce() + Send + 'static,
    {
        let stack = MmapAnon::new(stack_bytes, /*with_guard_page=*/ true)?;
        let closure: Box<FiberEntry> = Box::new(f);
        let mut fiber = Self {
            context: ManuallyDrop::new(Context::uninit()),
            stack: ManuallyDrop::new(stack),
            closure: Some(closure),
        };
        // SAFETY: `closure` is owned by `fiber` and will not be dropped
        // until the trampoline returns or the fiber is dropped.
        let closure_ptr: *mut Box<FiberEntry> = fiber.closure.as_mut().unwrap() as *mut _;
        let stack_top = unsafe { fiber.stack.as_ptr().add(fiber.stack.usable_bytes()) };
        // SAFETY: `Context::init` writes a stub frame onto the new
        // stack, sets the saved instruction pointer to the trampoline,
        // and loads the first argument with the closure pointer. The
        // trampoline reads the closure box, calls it, then traps via
        // `abort` (fiber re-entry is unsupported).
        unsafe {
            imp::init_context(
                &mut *fiber.context,
                stack_top,
                fiber_trampoline,
                closure_ptr as *mut u8,
            );
        }
        Ok(fiber)
    }

    /// Returns a raw pointer to this fiber's saved [`Context`]. The
    /// pointer is valid until the fiber is dropped.
    pub fn context(&self) -> *const Context {
        &*self.context as *const Context
    }

    /// Mutable counterpart to [`Fiber::context`]. Used by the caller's
    /// scheduler to switch *into* this fiber (the previous fiber's
    /// context is what receives the saved registers).
    pub fn context_mut(&mut self) -> *mut Context {
        &mut *self.context as *mut Context
    }
}

impl Drop for Fiber {
    fn drop(&mut self) {
        // ManuallyDrop is dropped manually in field order to guarantee
        // the saved context (which references stack memory) is dropped
        // before the stack itself.
        unsafe {
            ManuallyDrop::drop(&mut self.context);
            ManuallyDrop::drop(&mut self.stack);
        }
    }
}

// The entry trampoline. `closure_ptr` is a `*mut Box<FiberEntry>` that
// the trampoline lifts off the heap, calls, and then aborts the thread
// — fibers must not return into the scheduler from their entry without
// going through a cooperative switch, and we do not support implicit
// re-entry in Phase 3 PR 2.
extern "C" fn fiber_trampoline(closure_ptr: *mut u8) -> ! {
    // SAFETY: the per-arch `init_context` passes the same pointer we
    // stored in `Fiber::with_stack_size`, which points to the still-live
    // `Box<FiberEntry>` field of the owning `Fiber`.
    let boxed: Box<FiberEntry> = unsafe { std::ptr::read(closure_ptr as *const Box<FiberEntry>) };
    (boxed)();
    // The closure returned — there is no parent fiber to switch back to
    // in Phase 3 PR 2. Abort the process so the bug (forgotten switch)
    // is immediately visible.
    std::process::abort();
}
