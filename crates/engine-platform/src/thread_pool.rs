//! Owned N-worker thread pool (ADR-032).
//!
//! Each worker owns one OS thread spawned via `std::thread::spawn`. Job
//! submission goes through a per-worker FIFO deque (the worker's "local"
//! queue) plus a shared global injector; idle workers steal from peers
//! before parking on a condvar.
//!
//! No external scheduler crate is used (ADR-025): the queues are a
//! `Mutex<VecDeque<Job>>` apiece. Locking is per-pop / per-push and
//! contended only when several workers steal at once — within Phase 3's
//! workloads (handful of cores, ~10 ms jobs) the lock cost is below the
//! noise floor of the determinism contract. A genuinely lock-free deque
//! is a possible optimisation but not on Phase 3's critical path.
//!
//! # Allowlist
//!
//! This file is on the ADR-032 allowlist for `std::thread::spawn` and
//! `std::sync::Mutex` — the rest of the engine routes through this pool
//! instead of touching those primitives directly.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// A unit of work the pool can run. Boxed closure, FnOnce + Send.
pub type Job = Box<dyn FnOnce() + Send + 'static>;

struct Worker {
    queue: Mutex<std::collections::VecDeque<Job>>,
    park: Condvar,
}

impl Worker {
    fn new() -> Self {
        Self {
            queue: Mutex::new(std::collections::VecDeque::with_capacity(64)),
            park: Condvar::new(),
        }
    }
}

/// Shared state between every worker thread and the [`ThreadPool`] handle
/// the caller holds.
pub struct Pool {
    workers: Vec<Worker>,
    /// Global injector for jobs the caller submits with [`ThreadPool::dispatch`].
    injector: Mutex<std::collections::VecDeque<Job>>,
    /// Pending job counter. Incremented at submission, decremented after
    /// each job runs to completion. Workers and the `wait_idle` path
    /// observe this to decide when the pool is quiescent.
    pending: AtomicUsize,
    /// Round-robin counter for spreading injector submissions across
    /// worker queues.
    next_worker: AtomicUsize,
    /// Stop flag — true after `ThreadPool::shutdown`. Workers exit when
    /// they observe an empty queue and `stop = true`.
    stop: AtomicBool,
    /// Set by `wait_idle` to know when all queued work has drained.
    idle: Condvar,
    /// Sentinel `Mutex` guarding `idle`.
    idle_lock: Mutex<()>,
}

impl Pool {
    fn push_local(&self, worker_idx: usize, job: Job) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let mut q = self.workers[worker_idx].queue.lock().unwrap();
        q.push_back(job);
        drop(q);
        self.workers[worker_idx].park.notify_one();
    }

    fn push_global(&self, job: Job) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        let mut g = self.injector.lock().unwrap();
        g.push_back(job);
        drop(g);
        // Wake at least one worker.
        for w in self.workers.iter() {
            w.park.notify_one();
        }
    }

    fn pop_local(&self, worker_idx: usize) -> Option<Job> {
        let mut q = self.workers[worker_idx].queue.lock().unwrap();
        q.pop_front()
    }

    fn pop_global(&self) -> Option<Job> {
        let mut g = self.injector.lock().unwrap();
        g.pop_front()
    }

    fn steal(&self, my_idx: usize) -> Option<Job> {
        let n = self.workers.len();
        for off in 1..n {
            let victim = (my_idx + off) % n;
            let mut q = self.workers[victim].queue.lock().unwrap();
            if let Some(job) = q.pop_back() {
                return Some(job);
            }
        }
        None
    }

    fn job_done(&self) {
        let prev = self.pending.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Just transitioned to idle — notify any wait_idle waiter.
            let _guard = self.idle_lock.lock().unwrap();
            self.idle.notify_all();
        }
    }
}

/// Owned worker thread pool. Drops join every worker on shutdown.
pub struct ThreadPool {
    inner: Arc<Pool>,
    workers: Vec<JoinHandle<()>>,
}

impl ThreadPool {
    /// Builds a pool with `worker_count` workers.
    pub fn with_workers(worker_count: usize) -> Self {
        let n = worker_count.max(1);
        let mut worker_slots = Vec::with_capacity(n);
        for _ in 0..n {
            worker_slots.push(Worker::new());
        }
        let inner = Arc::new(Pool {
            workers: worker_slots,
            injector: Mutex::new(std::collections::VecDeque::with_capacity(64)),
            pending: AtomicUsize::new(0),
            next_worker: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
            idle: Condvar::new(),
            idle_lock: Mutex::new(()),
        });
        let mut handles = Vec::with_capacity(n);
        for idx in 0..n {
            let pool = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(format!("engine-job-{idx}"))
                .spawn(move || worker_loop(idx, pool))
                .expect("spawn worker thread");
            handles.push(handle);
        }
        Self {
            inner,
            workers: handles,
        }
    }

    /// Builds a pool sized to [`std::thread::available_parallelism`].
    pub fn with_default_workers() -> Self {
        let n = thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self::with_workers(n)
    }

    /// Submits a job to the pool. The job runs on some worker; ordering
    /// across submissions is unspecified beyond ADR-032's R/W-disjoint
    /// commutativity guarantee.
    pub fn dispatch<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        // Distribute via round-robin to a worker queue. The injector is
        // reserved for "global" jobs the topological dispatcher submits;
        // adjusting the policy here is harmless from the ABI's
        // perspective.
        let idx = self.inner.next_worker.fetch_add(1, Ordering::Relaxed) % self.inner.workers.len();
        self.inner.push_local(idx, Box::new(job));
    }

    /// Submits a job to the global injector (round-robined later by the
    /// next idle worker). Used by the [`crate::jobs::JobGraph`]
    /// dispatcher.
    pub fn dispatch_global<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner.push_global(Box::new(job));
    }

    /// Blocks until every previously submitted job has completed.
    pub fn wait_idle(&self) {
        let mut guard = self.inner.idle_lock.lock().unwrap();
        while self.inner.pending.load(Ordering::SeqCst) > 0 {
            guard = self.inner.idle.wait(guard).unwrap();
        }
    }

    /// The number of worker threads.
    pub fn worker_count(&self) -> usize {
        self.inner.workers.len()
    }

    /// Internal: clones the shared [`Pool`] handle. Used by
    /// [`crate::jobs::JobGraph::run_on`] to capture a `Send`-able
    /// reference into worker closures (an `Arc<Pool>` is `Send` even
    /// though `&ThreadPool` is not directly transferable through a
    /// borrow).
    pub(crate) fn pool_arc(&self) -> Arc<Pool> {
        Arc::clone(&self.inner)
    }
}

impl Pool {
    /// Public-in-crate variant of `push_global` so the `jobs` module
    /// can hand work straight to the injector without holding a
    /// borrow on a `ThreadPool` handle.
    pub(crate) fn submit(self: &Arc<Self>, job: Job) {
        self.push_global(job);
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        // Drain any remaining work first so the caller's submissions
        // aren't silently lost. Then signal stop and wake every worker.
        self.wait_idle();
        self.inner.stop.store(true, Ordering::SeqCst);
        for w in self.inner.workers.iter() {
            w.park.notify_all();
        }
        // Also wake any worker parked on the injector.
        // (notify_all on each worker's condvar suffices because workers
        // poll both their local queue and the injector before parking.)
        for handle in self.workers.drain(..) {
            // Don't propagate worker panics from shutdown — the pool is
            // already tearing down.
            let _ = handle.join();
        }
    }
}

fn worker_loop(worker_idx: usize, pool: Arc<Pool>) {
    loop {
        if let Some(job) = pool.pop_local(worker_idx) {
            job();
            pool.job_done();
            continue;
        }
        if let Some(job) = pool.pop_global() {
            job();
            pool.job_done();
            continue;
        }
        if let Some(job) = pool.steal(worker_idx) {
            job();
            pool.job_done();
            continue;
        }
        // Nothing to do: park until someone wakes us. Re-check the stop
        // flag and the queues after every wake.
        let mut q = pool.workers[worker_idx].queue.lock().unwrap();
        while q.is_empty()
            && pool.injector.lock().unwrap().is_empty()
            && !pool.stop.load(Ordering::SeqCst)
        {
            q = pool.workers[worker_idx].park.wait(q).unwrap();
        }
        if pool.stop.load(Ordering::SeqCst) && q.is_empty() {
            // Final exit only if no work remains.
            drop(q);
            if pool.injector.lock().unwrap().is_empty() && pool.pending.load(Ordering::SeqCst) == 0
            {
                return;
            }
            // Else loop and pick up the work.
        }
    }
}
