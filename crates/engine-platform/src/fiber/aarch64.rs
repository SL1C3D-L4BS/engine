//! AArch64 AAPCS64 fiber context switch (ADR-032).
//!
//! Saves the callee-saved general-purpose registers (`x19`-`x29`), `lr`,
//! `sp`, and the resume program counter; restores the same set from the
//! destination. The trampoline used at first entry receives its closure
//! argument in `x0` (AAPCS64 calling convention).

use core::arch::naked_asm;

/// Saved register snapshot for one fiber on AArch64.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    /// Saved `x19`.
    pub x19: u64,
    /// Saved `x20`.
    pub x20: u64,
    /// Saved `x21`.
    pub x21: u64,
    /// Saved `x22`.
    pub x22: u64,
    /// Saved `x23`.
    pub x23: u64,
    /// Saved `x24`.
    pub x24: u64,
    /// Saved `x25`.
    pub x25: u64,
    /// Saved `x26`.
    pub x26: u64,
    /// Saved `x27`.
    pub x27: u64,
    /// Saved `x28`.
    pub x28: u64,
    /// Saved `x29` (frame pointer).
    pub x29: u64,
    /// Saved `lr` / `x30` — the resume address.
    pub lr: u64,
    /// Saved stack pointer.
    pub sp: u64,
}

impl Context {
    /// Zero-initialised context. Must be initialised via [`init_context`]
    /// before being passed to [`switch`].
    pub const fn uninit() -> Self {
        Self {
            x19: 0,
            x20: 0,
            x21: 0,
            x22: 0,
            x23: 0,
            x24: 0,
            x25: 0,
            x26: 0,
            x27: 0,
            x28: 0,
            x29: 0,
            lr: 0,
            sp: 0,
        }
    }
}

/// Builds the initial context for a fresh fiber. The stack pointer is
/// aligned to a 16-byte boundary (AAPCS64 requires `sp` be 16-byte
/// aligned at function entry).
///
/// # Safety
///
/// `stack_top` must point one past the highest usable byte of the fiber
/// stack.
pub unsafe fn init_context(
    ctx: *mut Context,
    stack_top: *mut u8,
    entry: extern "C" fn(*mut u8) -> !,
    arg: *mut u8,
) {
    let mut sp = stack_top as usize;
    sp &= !0xF; // 16-byte align
    // SAFETY: the caller upholds that `stack_top` is in writable
    // memory; we are only initialising the saved-register block.
    unsafe {
        (*ctx).sp = sp as u64;
        (*ctx).lr = entry as usize as u64;
        // Ferry the trampoline argument through a callee-saved register;
        // the switch path moves it into `x0` on first entry.
        (*ctx).x19 = arg as u64;
    }
}

/// Switch from `prev` to `next`.
///
/// # Safety
///
/// See the module-level documentation of [`super::switch`].
#[unsafe(naked)]
pub unsafe extern "C" fn switch(prev: *mut Context, next: *const Context) {
    // SAFETY: `#[unsafe(naked)]` function; body is a single
    // `naked_asm!` block with explicit register saves/restores.
    naked_asm!(
        // Save callee-saved registers into prev (x0 = prev).
        "stp x19, x20, [x0, #0x00]",
        "stp x21, x22, [x0, #0x10]",
        "stp x23, x24, [x0, #0x20]",
        "stp x25, x26, [x0, #0x30]",
        "stp x27, x28, [x0, #0x40]",
        "stp x29, x30, [x0, #0x50]", // x29 = fp, x30 = lr
        "mov x2, sp",
        "str x2, [x0, #0x60]",
        // Load destination context (x1 = next).
        "ldp x19, x20, [x1, #0x00]",
        "ldp x21, x22, [x1, #0x10]",
        "ldp x23, x24, [x1, #0x20]",
        "ldp x25, x26, [x1, #0x30]",
        "ldp x27, x28, [x1, #0x40]",
        "ldp x29, x30, [x1, #0x50]",
        "ldr x2, [x1, #0x60]",
        "mov sp, x2",
        // First-entry: move trampoline argument from x19 to x0.
        // Harmless on subsequent switches (x0 is caller-saved).
        "mov x0, x19",
        // Resume at the loaded lr.
        "ret",
    );
}
