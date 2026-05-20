//! The system scheduler.
//!
//! A system is a function over the [`World`]. Systems are grouped into the
//! fixed [`Phase`]s and, within a phase, run in registration order. That order
//! is a stable topological sort keyed by `(phase, registration index)` — fully
//! deterministic across runs, as the Determinism Contract requires (spec IV.2).
//!
//! Phase 3 (ADR-033) adds [`Schedule::add_system_with_access`] and
//! [`Schedule::run_on`]: systems declare their read and write [`TypeStableId`]
//! sets at registration; [`run_on`] builds one [`JobGraph`] per phase and
//! dispatches non-conflicting systems through the owned thread pool. The
//! observable result is identical to the sequential [`run`] — the replay
//! parity oracle is the backstop against R/W declarations that lie.
//!
//! [`run`]: Schedule::run
//! [`run_on`]: Schedule::run_on
//! [`TypeStableId`]: super::TypeStableId
//! [`JobGraph`]: engine_platform::JobGraph

use super::{TypeStableId, World};
use engine_platform::{JobGraph, ThreadPool};

/// The ordered execution phases of a frame (spec IV.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Input, network receive, ECS housekeeping.
    PreUpdate,
    /// Gameplay, AI, physics.
    Update,
    /// Transform-hierarchy propagation, event drain.
    PostUpdate,
    /// Frustum cull, render-data extraction.
    PreRender,
    /// GPU submission.
    Render,
    /// Stats, debug overlays.
    PostRender,
}

impl Phase {
    /// Every phase, in execution order.
    pub const ALL: [Phase; 6] = [
        Phase::PreUpdate,
        Phase::Update,
        Phase::PostUpdate,
        Phase::PreRender,
        Phase::Render,
        Phase::PostRender,
    ];

    /// The phase's position in the frame (lower runs earlier).
    fn rank(self) -> u8 {
        match self {
            Phase::PreUpdate => 0,
            Phase::Update => 1,
            Phase::PostUpdate => 2,
            Phase::PreRender => 3,
            Phase::Render => 4,
            Phase::PostRender => 5,
        }
    }
}

/// Boxed system body. One alias so the `dispatch_phase` swap-in/out
/// machinery doesn't trip the `type_complexity` lint.
type SystemBody = Box<dyn FnMut(&mut World) + Send>;

struct SystemEntry {
    name: &'static str,
    phase: Phase,
    /// Declared component-id reads (ADR-033). Empty for the legacy
    /// `add_system` path; in that case the system is treated as
    /// exclusive (conflicts with every other system in its phase) so
    /// the parallel scheduler still produces deterministic output.
    reads: Vec<TypeStableId>,
    /// Declared component-id writes.
    writes: Vec<TypeStableId>,
    /// `true` when the system was registered via `add_system` (no R/W
    /// declaration). Parallel dispatch falls back to single-threaded
    /// for the whole phase if any exclusive system is present, mirroring
    /// the sequential ordering exactly.
    exclusive: bool,
    run: SystemBody,
}

/// An ordered collection of systems, executed once per frame by [`run`].
///
/// [`run`]: Schedule::run
#[derive(Default)]
pub struct Schedule {
    systems: Vec<SystemEntry>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an exclusive system in `phase`. Returns `&mut Self` for
    /// chaining.
    ///
    /// Exclusive systems take `&mut World` without declaring which
    /// components they touch; [`run_on`] therefore can't parallelise them
    /// against anything else in the same phase, and falls back to
    /// single-threaded execution if any phase contains an exclusive
    /// system. Use [`add_system_with_access`] for parallelisable
    /// systems.
    ///
    /// [`run_on`]: Self::run_on
    /// [`add_system_with_access`]: Self::add_system_with_access
    pub fn add_system(
        &mut self,
        phase: Phase,
        name: &'static str,
        run: impl FnMut(&mut World) + Send + 'static,
    ) -> &mut Self {
        self.systems.push(SystemEntry {
            name,
            phase,
            reads: Vec::new(),
            writes: Vec::new(),
            exclusive: true,
            run: Box::new(run),
        });
        self
    }

    /// Registers a parallelisable system in `phase`, with explicit
    /// component-id read and write sets (ADR-033).
    ///
    /// The R/W sets feed the per-phase [`JobGraph`] built by [`run_on`]:
    /// two systems whose R/W sets are pairwise disjoint *except* on
    /// reads run in parallel; any write/write or write/read overlap
    /// serialises the pair in registration order.
    ///
    /// The replay-parity oracle (`tests/replay_parity.rs`) is the
    /// backstop against R/W declarations that lie — if a system reads
    /// or writes something it didn't declare, the digest will diverge
    /// across worker counts.
    ///
    /// [`JobGraph`]: engine_platform::JobGraph
    /// [`run_on`]: Self::run_on
    pub fn add_system_with_access(
        &mut self,
        phase: Phase,
        name: &'static str,
        reads: &[TypeStableId],
        writes: &[TypeStableId],
        run: impl FnMut(&mut World) + Send + 'static,
    ) -> &mut Self {
        self.systems.push(SystemEntry {
            name,
            phase,
            reads: reads.to_vec(),
            writes: writes.to_vec(),
            exclusive: false,
            run: Box::new(run),
        });
        self
    }

    /// The number of registered systems.
    pub fn system_count(&self) -> usize {
        self.systems.len()
    }

    /// The stable execution order: `(name, phase)` for each system, sorted by
    /// `(phase rank, registration index)`.
    pub fn execution_order(&self) -> Vec<(&'static str, Phase)> {
        self.ordered_indices()
            .into_iter()
            .map(|i| (self.systems[i].name, self.systems[i].phase))
            .collect()
    }

    /// Runs every system once, in [`execution_order`](Self::execution_order).
    pub fn run(&mut self, world: &mut World) {
        for i in self.ordered_indices() {
            (self.systems[i].run)(world);
        }
    }

    /// Runs every system once, dispatching non-conflicting systems through
    /// `pool` (ADR-033).
    ///
    /// Each [`Phase`] is dispatched as one [`JobGraph`]; phases run
    /// strictly in [`Phase::ALL`] order. The resulting `World` state is
    /// identical to [`run`] when every system's declared R/W set is
    /// honest — the replay-parity oracle verifies that.
    ///
    /// [`run`]: Self::run
    /// [`JobGraph`]: engine_platform::JobGraph
    pub fn run_on(&mut self, world: &mut World, pool: &ThreadPool) {
        for &phase in Phase::ALL.iter() {
            let in_phase: Vec<usize> = self
                .systems
                .iter()
                .enumerate()
                .filter(|(_, s)| s.phase == phase)
                .map(|(i, _)| i)
                .collect();
            if in_phase.is_empty() {
                continue;
            }
            // Exclusive systems fall back to single-threaded execution
            // for the whole phase. The semantics match `run` exactly.
            let any_exclusive = in_phase.iter().any(|&i| self.systems[i].exclusive);
            if any_exclusive {
                for i in in_phase {
                    (self.systems[i].run)(world);
                }
                continue;
            }
            self.dispatch_phase(&in_phase, world, pool);
        }
    }

    /// Builds the [`JobGraph`] for one phase and runs it on `pool`.
    fn dispatch_phase(&mut self, in_phase: &[usize], world: &mut World, pool: &ThreadPool) {
        // SAFETY contract for the worker closures below:
        //
        // Every system in this phase declared its R/W [`TypeStableId`]
        // set up-front. The JobGraph computes successors via the R/W
        // conflict rule (two reads commute; anything touching a written
        // key serialises). When a system's closure body actually only
        // touches the components it declared, no two parallel jobs
        // alias the same component memory, so the `&mut World` they
        // each receive may be reborrowed safely. This holds across the
        // whole `World` because:
        //
        // - Table-component columns live in per-archetype `AnyVec`s
        //   keyed by `TypeStableId`. Two jobs that don't share a key
        //   never touch the same column.
        // - Sparse-component columns are keyed the same way.
        // - The entity allocator and archetype index are never
        //   structurally mutated by parallel systems (insert / despawn
        //   require exclusive access; those systems are registered via
        //   `add_system` and route through the sequential fallback
        //   above).
        //
        // The replay-parity oracle (ADR-033) is the runtime backstop
        // against a system whose declaration lies — divergent worker
        // counts produce divergent digests.
        let world_ptr = WorldPtr(world as *mut World);

        // Per-system closure handles. We move each FnMut out of
        // `self.systems[i].run` into the JobGraph via a one-shot Mutex
        // wrapper, then put it back when the phase finishes — this
        // keeps `run_on` re-callable for the next frame without
        // requiring the closures to be `Clone`.
        let mut bodies: Vec<Option<SystemBody>> = in_phase.iter().map(|_| None).collect();
        for (slot, &i) in in_phase.iter().enumerate() {
            // Temporarily replace the system body with a no-op stub;
            // restored after dispatch.
            let stub: SystemBody = Box::new(|_| {});
            bodies[slot] = Some(std::mem::replace(&mut self.systems[i].run, stub));
        }

        let bodies = std::sync::Arc::new(std::sync::Mutex::new(bodies));

        let mut graph = JobGraph::new();
        for slot in 0..in_phase.len() {
            let i = in_phase[slot];
            let reads: Vec<u64> = self.systems[i].reads.iter().map(|id| id.as_u64()).collect();
            let writes: Vec<u64> = self.systems[i]
                .writes
                .iter()
                .map(|id| id.as_u64())
                .collect();
            let bodies = std::sync::Arc::clone(&bodies);
            graph.add_job(&reads, &writes, move || {
                // SAFETY: see the contract block above. Each closure
                // touches only its declared R/W set; non-conflicting
                // jobs run in parallel without aliasing.
                //
                // `world_ptr.get()` (vs `world_ptr.0`) forces the
                // Rust 2021 disjoint-capture analyser to capture the
                // whole `WorldPtr` (which is `Send + Sync` by unsafe
                // impl below) rather than just the inner `*mut World`
                // field (which is not).
                let world: &mut World = unsafe { &mut *world_ptr.get() };
                let mut guard = bodies.lock().expect("bodies mutex");
                let mut body = guard[slot].take().expect("body present");
                drop(guard);
                body(world);
                bodies.lock().expect("bodies mutex")[slot] = Some(body);
            });
        }
        graph.run_on(pool);

        // Restore the system bodies.
        let mut guard = bodies.lock().expect("bodies mutex");
        for (slot, &i) in in_phase.iter().enumerate() {
            self.systems[i].run = guard[slot].take().expect("body restored");
        }
    }

    fn ordered_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.systems.len()).collect();
        // A stable sort on phase rank preserves registration order within a
        // phase — the deterministic tiebreak the contract requires.
        indices.sort_by_key(|&i| self.systems[i].phase.rank());
        indices
    }
}

/// Wrapper around `*mut World` that is `Send + Sync` for transfer through
/// the [`JobGraph`] dispatch path (ADR-033). The unsafety is justified in
/// [`Schedule::dispatch_phase`].
#[derive(Clone, Copy)]
struct WorldPtr(*mut World);

impl WorldPtr {
    /// Returns the inner raw pointer. Wrapped so that closures capture
    /// the [`WorldPtr`] (Send) rather than just the inner field
    /// (which is `*mut World` and not `Send` under Rust 2021's
    /// disjoint-capture rules).
    fn get(self) -> *mut World {
        self.0
    }
}

// SAFETY: see `Schedule::dispatch_phase`.
unsafe impl Send for WorldPtr {}
// SAFETY: see `Schedule::dispatch_phase`.
unsafe impl Sync for WorldPtr {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systems_run_in_phase_then_registration_order() {
        let mut schedule = Schedule::new();
        schedule
            .add_system(Phase::Render, "render", |_| {})
            .add_system(Phase::PreUpdate, "input", |_| {})
            .add_system(Phase::Update, "physics", |_| {})
            .add_system(Phase::Update, "ai", |_| {});

        let order = schedule.execution_order();
        assert_eq!(
            order,
            vec![
                ("input", Phase::PreUpdate),
                ("physics", Phase::Update),
                ("ai", Phase::Update),
                ("render", Phase::Render),
            ]
        );
    }

    #[test]
    fn order_is_stable_across_calls() {
        let mut schedule = Schedule::new();
        for _ in 0..50 {
            schedule.add_system(Phase::Update, "s", |_| {});
        }
        assert_eq!(schedule.execution_order(), schedule.execution_order());
    }

    #[test]
    fn run_executes_every_system() {
        let mut world = World::new();
        world.insert_resource(0u32);
        let mut schedule = Schedule::new();
        schedule.add_system(Phase::Update, "tick", |w: &mut World| {
            *w.resource_mut::<u32>().unwrap() += 1;
        });
        schedule.run(&mut world);
        schedule.run(&mut world);
        assert_eq!(*world.resource::<u32>().unwrap(), 2);
    }
}
