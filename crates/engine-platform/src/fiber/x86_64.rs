//! x86-64 System V fiber context switch (ADR-032).
//!
//! Saves the callee-saved registers (`rbx`, `rbp`, `r12`-`r15`), the stack
//! pointer, and the resume instruction pointer; restores the same set from
//! the destination context. The trampoline used at first entry receives
//! its closure argument in `rdi` (System V calling convention).

use core::arch::naked_asm;

/// Saved register snapshot for one fiber. Layout is fixed by the offsets
/// in the naked asm below — do not reorder.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Context {
    /// Saved `rbx`.
    pub rbx: u64,
    /// Saved `rbp`.
    pub rbp: u64,
    /// Saved `r12`.
    pub r12: u64,
    /// Saved `r13`.
    pub r13: u64,
    /// Saved `r14`.
    pub r14: u64,
    /// Saved `r15`.
    pub r15: u64,
    /// Saved stack pointer (`rsp`).
    pub rsp: u64,
    /// Saved instruction pointer (`rip`) — where to resume execution
    /// when the fiber is next switched into.
    pub rip: u64,
}

impl Context {
    /// Zero-initialised context. Must be initialised via [`init_context`]
    /// before being passed to [`switch`].
    pub const fn uninit() -> Self {
        Self {
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rsp: 0,
            rip: 0,
        }
    }
}

/// Builds the initial context for a fresh fiber. Lays out the bottom of
/// the stack with one slot for an "alignment + return pseudo-frame" so
/// the trampoline reads its first argument from `rdi` (set up below) and
/// finds a 16-byte-aligned `rsp` on entry — the System V ABI requires
/// `rsp % 16 == 8` at function entry (the call instruction would have
/// pushed a return address bringing the offset to 0, so we mimic that by
/// reserving 8 bytes).
///
/// # Safety
///
/// `stack_top` must be a writable pointer one past the highest usable
/// address of the fiber's stack; this function writes a small fixed
/// number of bytes below it.
pub unsafe fn init_context(
    ctx: *mut Context,
    stack_top: *mut u8,
    entry: extern "C" fn(*mut u8) -> !,
    arg: *mut u8,
) {
    // Align the stack to 16 bytes; reserve 8 bytes for the implicit
    // return-address slot (System V's `rsp % 16 == 8` at function entry
    // invariant). The trampoline never returns, so the slot is dead.
    let mut sp = stack_top as usize;
    sp &= !0xF; // 16-byte align
    sp -= 8; // pretend there's a return address pushed
    // SAFETY: the caller upholds that `stack_top` is in writable memory;
    // we have only touched the top of the stack so far.
    unsafe {
        (*ctx).rsp = sp as u64;
        (*ctx).rip = entry as usize as u64;
        // The trampoline reads its argument from `rdi`; we stash it in
        // `rbx` and have the switch path move it into `rdi` on first
        // entry. Easier: use `r12` (callee-saved) to ferry the arg
        // through the switch, then a small wrapper moves it into `rdi`.
        //
        // For simplicity, we keep the closure pointer in `rbx`. The
        // first-entry path below moves it into `rdi`. See `switch`.
        (*ctx).rbx = arg as u64;
        // Initial register state is otherwise zero — `rbp = 0` makes
        // backtraces stop cleanly at the trampoline.
    }
}

/// Switch from `prev` to `next`. Naked asm: saves callee-saved + rsp +
/// rip into `prev`, then loads the same from `next`. On first entry the
/// trampoline argument is moved from `rbx` to `rdi`.
///
/// # Safety
///
/// See the module-level documentation of [`super::switch`].
#[unsafe(naked)]
pub unsafe extern "C" fn switch(prev: *mut Context, next: *const Context) {
    // SAFETY: this function is `#[unsafe(naked)]`; the body must consist
    // of a single `naked_asm!` invocation and may not access any
    // Rust-level locals other than the parameters.
    naked_asm!(
        // Save current callee-saved registers + rsp + rip into prev.
        "mov [rdi + 0x00], rbx",
        "mov [rdi + 0x08], rbp",
        "mov [rdi + 0x10], r12",
        "mov [rdi + 0x18], r13",
        "mov [rdi + 0x20], r14",
        "mov [rdi + 0x28], r15",
        "mov [rdi + 0x30], rsp",
        // The resume IP is the return address on top of the stack —
        // the call instruction pushed it. Pull it into rax then store.
        "mov rax, [rsp]",
        "mov [rdi + 0x38], rax",
        // Load destination context.
        "mov rbx, [rsi + 0x00]",
        "mov rbp, [rsi + 0x08]",
        "mov r12, [rsi + 0x10]",
        "mov r13, [rsi + 0x18]",
        "mov r14, [rsi + 0x20]",
        "mov r15, [rsi + 0x28]",
        "mov rsp, [rsi + 0x30]",
        // On first entry the trampoline expects its argument in rdi.
        // We ferried it through rbx; move it across. For subsequent
        // switches `rdi` is irrelevant (callee-saved registers carry
        // the live state), so the unconditional move is harmless.
        "mov rdi, rbx",
        // Jump to the saved IP, replacing the original return.
        "mov rax, [rsi + 0x38]",
        "jmp rax",
    );
}
