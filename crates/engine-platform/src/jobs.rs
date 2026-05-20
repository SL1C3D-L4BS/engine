//! Static-DAG job graph + dependency-aware dispatch (ADR-032).
//!
//! A [`JobGraph`] is built by the caller in two passes: register jobs with
//! their R/W component sets via [`JobGraph::add_job`], then call
//! [`JobGraph::run`] (single-threaded reference path) or
//! [`JobGraph::run_on`] (parallel dispatch through a [`ThreadPool`]).
//!
//! Dependency rule (ADR-032): jobs `A` and `B` conflict iff one of them
//! writes a stable id that the other reads or writes. Two read-only
//! accesses do *not* conflict. Within a conflict pair, dependency edges
//! go from the earlier-registered job to the later one — the registration
//! index is the deterministic tiebreak the contract requires.
//!
//! The graph is not generic in the access-key type — it carries
//! `u64` keys. Callers (e.g. the Phase 3 `engine_core::Schedule`) feed
//! `TypeStableId::as_u64()` into it; the pool sees only opaque integers.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::thread_pool::{Pool, ThreadPool};

/// Type alias for the per-job boxed closure storage, used in the
/// `JobGraph::run_on` dispatcher. Single named type so the
/// `type_complexity` lint is happy and the `spawn_job` signature is
/// readable.
type BodyStore = Arc<Vec<Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>>>;

/// Identifier of a job within a graph. Allocated densely from `0` in
/// registration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(pub u32);

struct JobEntry {
    /// Sorted, deduplicated keys the job reads.
    reads: Vec<u64>,
    /// Sorted, deduplicated keys the job writes.
    writes: Vec<u64>,
    /// Boxed closure. `Option` so [`JobGraph::run`] can move it out by
    /// calling `.take()`.
    body: Option<Box<dyn FnOnce() + Send + 'static>>,
}

/// A static DAG of jobs with R/W access declarations.
///
/// Construction is single-threaded; dispatch ([`run`](Self::run) or
/// [`run_on`](Self::run_on)) consumes the graph (jobs run exactly once).
pub struct JobGraph {
    jobs: Vec<JobEntry>,
}

impl Default for JobGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl JobGraph {
    /// Builds an empty graph.
    pub fn new() -> Self {
        Self { jobs: Vec::new() }
    }

    /// Registers a job with the keys it reads and the keys it writes.
    /// Returns the job's [`JobId`].
    pub fn add_job<F>(&mut self, reads: &[u64], writes: &[u64], body: F) -> JobId
    where
        F: FnOnce() + Send + 'static,
    {
        let id = JobId(self.jobs.len() as u32);
        let mut rs = reads.to_vec();
        rs.sort_unstable();
        rs.dedup();
        let mut ws = writes.to_vec();
        ws.sort_unstable();
        ws.dedup();
        self.jobs.push(JobEntry {
            reads: rs,
            writes: ws,
            body: Some(Box::new(body)),
        });
        id
    }

    /// The number of jobs registered.
    pub fn job_count(&self) -> usize {
        self.jobs.len()
    }

    /// Single-threaded reference execution: runs every job in
    /// registration order. The oracle uses this as the "ground truth"
    /// to compare against parallel runs.
    pub fn run(mut self) {
        for entry in self.jobs.iter_mut() {
            let body = entry
                .body
                .take()
                .expect("each job runs exactly once per graph");
            body();
        }
    }

    /// Parallel execution on `pool`, respecting the R/W dependency rule
    /// (ADR-032). Blocks until every job has run; the pool stays open
    /// for subsequent graphs.
    ///
    /// The dispatcher constructs the in-degree array per job, kicks off
    /// every zero-in-degree job, and decrements successors as each job
    /// completes. When the last job finishes, the caller's wait condvar
    /// fires.
    #[allow(clippy::needless_range_loop)]
    pub fn run_on(self, pool: &ThreadPool) {
        let n = self.jobs.len();
        if n == 0 {
            return;
        }
        // 1. Compute successors and in-degree. Indexed access — both
        // arrays need mutation at distinct indices in the same iteration.
        let mut successors: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];
        for i in 0..n {
            for j in (i + 1)..n {
                if conflict(&self.jobs[i], &self.jobs[j]) {
                    successors[i].push(j);
                    in_degree[j] += 1;
                }
            }
        }

        // 2. Snapshot the initial zero-in-degree set *before* the
        // in-degree vector becomes shared / atomic. Otherwise the
        // initial scan would race with the first batch of workers'
        // recursive spawn_jobs: a worker could decrement `in_deg[j]`
        // from 1 to 0 and spawn `j`, while the main thread's scan
        // could *also* observe `in_deg[j] == 0` and spawn `j` a
        // second time — double-take on `bodies[j]` then panics with
        // "job runs once".
        let zero_initial: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();

        // 3. Shared state: per-job in-degree (atomic so workers can
        // decrement), a remaining-job counter, and a condvar for the
        // submitter to wait on.
        let remaining = Arc::new(AtomicUsize::new(n));
        let in_deg: Arc<Vec<AtomicUsize>> =
            Arc::new(in_degree.into_iter().map(AtomicUsize::new).collect());
        let bodies: BodyStore =
            Arc::new(self.jobs.into_iter().map(|e| Mutex::new(e.body)).collect());
        let successors = Arc::new(successors);
        let done_lock = Arc::new(Mutex::new(()));
        let done_cv = Arc::new(Condvar::new());

        let pool_arc = pool.pool_arc();
        // 4. Kick off the pre-computed zero-in-degree set.
        for i in zero_initial {
            spawn_job(
                i,
                &in_deg,
                &successors,
                &bodies,
                &remaining,
                &done_lock,
                &done_cv,
                &pool_arc,
            );
        }

        // 5. Wait for completion.
        let mut guard = done_lock.lock().unwrap();
        while remaining.load(Ordering::SeqCst) > 0 {
            guard = done_cv.wait(guard).unwrap();
        }
    }
}

fn conflict(a: &JobEntry, b: &JobEntry) -> bool {
    // a writes that b reads or writes — OR — b writes that a reads or
    // writes. Sorted slices → O(n+m) merge would suffice; for the
    // graph sizes Phase 3 exercises (dozens of systems), the naive
    // nested loop is faster than constructing temporary merge state.
    fn intersects(xs: &[u64], ys: &[u64]) -> bool {
        let (mut i, mut j) = (0, 0);
        while i < xs.len() && j < ys.len() {
            match xs[i].cmp(&ys[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }
    intersects(&a.writes, &b.reads)
        || intersects(&a.writes, &b.writes)
        || intersects(&a.reads, &b.writes)
}

#[allow(clippy::too_many_arguments)]
fn spawn_job(
    i: usize,
    in_deg: &Arc<Vec<AtomicUsize>>,
    successors: &Arc<Vec<Vec<usize>>>,
    bodies: &BodyStore,
    remaining: &Arc<AtomicUsize>,
    done_lock: &Arc<Mutex<()>>,
    done_cv: &Arc<Condvar>,
    pool: &Arc<Pool>,
) {
    let in_deg = Arc::clone(in_deg);
    let successors = Arc::clone(successors);
    let bodies = Arc::clone(bodies);
    let remaining = Arc::clone(remaining);
    let done_lock = Arc::clone(done_lock);
    let done_cv = Arc::clone(done_cv);
    let pool_for_submit = Arc::clone(pool);
    let pool_for_inner = Arc::clone(pool);

    let job: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
        // Run the job.
        let body = bodies[i].lock().unwrap().take().expect("job runs once");
        body();

        // Decrement successors and re-spawn any that hit zero in-degree.
        let succ = &successors[i];
        for &j in succ.iter() {
            let prev = in_deg[j].fetch_sub(1, Ordering::SeqCst);
            if prev == 1 {
                spawn_job(
                    j,
                    &in_deg,
                    &successors,
                    &bodies,
                    &remaining,
                    &done_lock,
                    &done_cv,
                    &pool_for_inner,
                );
            }
        }

        // Final accounting.
        let left = remaining.fetch_sub(1, Ordering::SeqCst);
        if left == 1 {
            let _g = done_lock.lock().unwrap();
            done_cv.notify_all();
        }
    });
    pool_for_submit.submit(job);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn empty_graph_returns_immediately() {
        let graph = JobGraph::new();
        graph.run();
    }

    #[test]
    fn single_job_runs() {
        let mut graph = JobGraph::new();
        let n = Arc::new(AtomicU64::new(0));
        let n2 = Arc::clone(&n);
        graph.add_job(&[], &[1], move || {
            n2.fetch_add(1, Ordering::SeqCst);
        });
        graph.run();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn read_only_jobs_dont_conflict() {
        let a = JobEntry {
            reads: vec![1, 2],
            writes: vec![],
            body: None,
        };
        let b = JobEntry {
            reads: vec![2, 3],
            writes: vec![],
            body: None,
        };
        assert!(!conflict(&a, &b));
    }

    #[test]
    fn write_read_pair_conflicts() {
        let a = JobEntry {
            reads: vec![],
            writes: vec![1],
            body: None,
        };
        let b = JobEntry {
            reads: vec![1],
            writes: vec![],
            body: None,
        };
        assert!(conflict(&a, &b));
    }

    #[test]
    fn write_write_pair_conflicts() {
        let a = JobEntry {
            reads: vec![],
            writes: vec![5],
            body: None,
        };
        let b = JobEntry {
            reads: vec![],
            writes: vec![5],
            body: None,
        };
        assert!(conflict(&a, &b));
    }
}
