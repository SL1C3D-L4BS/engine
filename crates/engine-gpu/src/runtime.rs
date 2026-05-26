//! Tiny in-crate executor for polling `wgpu`'s `request_adapter` / `request_device`
//! futures synchronously. The engine has no async runtime and never will (the
//! owned-discipline answer to spec §0.3 R-03's "no third-party async runtime");
//! the wgpu futures are GPU-driven and complete in microseconds, so a busy-yield
//! poll is the correct shape here.
//!
//! Not exposed outside the crate. Used by [`crate::device::Device::new`].

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};

/// Drive `fut` to completion on the current thread.
///
/// Polling uses [`Waker::noop`] (stable since Rust 1.85; the workspace pins
/// 1.95). Between polls the thread yields rather than spins, so the scheduler
/// can wake the wgpu worker that resolves the future.
pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}
