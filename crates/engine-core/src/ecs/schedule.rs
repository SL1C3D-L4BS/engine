//! The system scheduler.
//!
//! A system is a function over the [`World`]. Systems are grouped into the
//! fixed [`Phase`]s and, within a phase, run in registration order. That order
//! is a stable topological sort keyed by `(phase, registration index)` — fully
//! deterministic across runs, as the Determinism Contract requires (spec IV.2).
//!
//! Parallel execution with static read/write dependency analysis (running
//! non-conflicting systems concurrently) is the Phase 3 performance rewrite;
//! it does not change the *observable* order, because parallel systems write
//! disjoint memory.

use super::World;

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

struct SystemEntry {
    name: &'static str,
    phase: Phase,
    run: Box<dyn FnMut(&mut World)>,
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

    /// Registers a system in `phase`. Returns `&mut Self` for chaining.
    pub fn add_system(
        &mut self,
        phase: Phase,
        name: &'static str,
        run: impl FnMut(&mut World) + 'static,
    ) -> &mut Self {
        self.systems.push(SystemEntry {
            name,
            phase,
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

    fn ordered_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = (0..self.systems.len()).collect();
        // A stable sort on phase rank preserves registration order within a
        // phase — the deterministic tiebreak the contract requires.
        indices.sort_by_key(|&i| self.systems[i].phase.rank());
        indices
    }
}

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
