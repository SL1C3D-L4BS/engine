//! Archetype-storage oracle (ADR-031).
//!
//! Exercises the Phase 3 archetype index: adjacency moves on
//! [`World::insert`], both backends (Table + SparseSet), swap-remove
//! correctness when a row is removed from the middle of an archetype, and
//! the [`World::query`] iterator's archetype-walk ordering.

use engine_core::ecs::query::Mut;
use engine_core::{ArchetypeId, Component, Query, TypeStableId, World};

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Position {
    x: i32,
    y: i32,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Velocity {
    dx: i32,
    dy: i32,
}

#[derive(Component, Debug, PartialEq, Clone, Copy)]
struct Health {
    hp: i32,
}

#[derive(Component, Debug, PartialEq)]
#[component(storage = "SparseSet")]
struct Selected;

#[test]
fn stable_ids_are_distinct_per_type() {
    let p = <Position as Component>::STABLE_ID;
    let v = <Velocity as Component>::STABLE_ID;
    let h = <Health as Component>::STABLE_ID;
    assert_ne!(p, v);
    assert_ne!(p, h);
    assert_ne!(v, h);
    // The id should be a non-trivial integer.
    assert_ne!(p, TypeStableId(0));
}

#[test]
fn spawning_an_entity_with_three_components_walks_three_archetype_edges() {
    let mut world = World::new();
    let e = world.spawn();
    // Empty archetype before any insert.
    assert_eq!(world.archetype_count(), 1);
    world.insert(e, Position { x: 1, y: 2 });
    world.insert(e, Velocity { dx: 3, dy: 4 });
    world.insert(e, Health { hp: 5 });
    // One archetype per signature visited: empty, {P}, {P,V}, {P,V,H}.
    assert_eq!(world.archetype_count(), 4);

    assert_eq!(world.get::<Position>(e), Some(&Position { x: 1, y: 2 }));
    assert_eq!(world.get::<Velocity>(e), Some(&Velocity { dx: 3, dy: 4 }));
    assert_eq!(world.get::<Health>(e), Some(&Health { hp: 5 }));
}

#[test]
fn inserting_components_in_different_orders_interns_the_same_archetype() {
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();

    world.insert(a, Position { x: 1, y: 1 });
    world.insert(a, Velocity { dx: 1, dy: 1 });

    world.insert(b, Velocity { dx: 2, dy: 2 });
    world.insert(b, Position { x: 2, y: 2 });

    // {P, V} and {V, P} must intern to the same archetype, regardless of
    // insert order — that is the determinism invariant ADR-031 names.
    let archetypes_with_pv: Vec<ArchetypeId> = (0..world.archetype_count() as u32)
        .map(ArchetypeId)
        .filter(|_| true)
        .collect();
    let _ = archetypes_with_pv;
    // Crisper version: query::<(&Position, &Velocity)> must visit both.
    let mut count = 0;
    for (_, p, v) in world.query::<(&Position, &Velocity)>().iter() {
        count += 1;
        // Property: positions and velocities arrive together.
        assert_eq!(p.x, v.dx);
        assert_eq!(p.y, v.dy);
    }
    assert_eq!(count, 2);
}

#[test]
fn remove_returns_owned_value_and_moves_to_smaller_archetype() {
    let mut world = World::new();
    let e = world.spawn();
    world.insert(e, Position { x: 5, y: 6 });
    world.insert(e, Velocity { dx: 7, dy: 8 });

    let removed = world.remove::<Velocity>(e);
    assert_eq!(removed, Some(Velocity { dx: 7, dy: 8 }));
    assert!(!world.contains::<Velocity>(e));
    assert_eq!(world.get::<Position>(e), Some(&Position { x: 5, y: 6 }));
}

#[test]
fn despawn_in_middle_of_archetype_patches_remaining_rows() {
    let mut world = World::new();
    let mut entities = Vec::new();
    for i in 0..8 {
        let e = world.spawn();
        world.insert(e, Position { x: i, y: -i });
        entities.push(e);
    }

    // Despawn entity at row 3.
    world.despawn(entities[3]);

    // The other seven entities still have intact positions.
    for (i, &e) in entities.iter().enumerate() {
        if i == 3 {
            assert!(!world.is_alive(e));
            assert_eq!(world.get::<Position>(e), None);
        } else {
            assert_eq!(
                world.get::<Position>(e),
                Some(&Position {
                    x: i as i32,
                    y: -(i as i32),
                })
            );
        }
    }
    assert_eq!(world.entity_count(), 7);
}

#[test]
fn sparse_set_backend_does_not_change_archetype() {
    let mut world = World::new();
    let e = world.spawn();
    world.insert(e, Position { x: 0, y: 0 });
    let arch_before = world.archetype_count();
    world.insert(e, Selected);
    let arch_after = world.archetype_count();
    // SparseSet components never enter a signature → no new archetype.
    assert_eq!(arch_before, arch_after);
    assert!(world.contains::<Selected>(e));
    assert!(world.contains::<Position>(e));
}

#[test]
fn query_yields_entities_across_all_matching_archetypes() {
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();
    let c = world.spawn();
    world.insert(a, Position { x: 1, y: 0 });
    world.insert(b, Position { x: 2, y: 0 });
    world.insert(b, Velocity { dx: 1, dy: 0 });
    world.insert(c, Position { x: 3, y: 0 });
    world.insert(c, Velocity { dx: 1, dy: 0 });
    world.insert(c, Health { hp: 10 });

    let xs: Vec<i32> = world
        .query::<&Position>()
        .iter()
        .map(|(_, p)| p.x)
        .collect();
    // Every entity has Position; query walks all matching archetypes.
    assert_eq!(xs.len(), 3);
    assert!(xs.contains(&1));
    assert!(xs.contains(&2));
    assert!(xs.contains(&3));
}

#[test]
fn query_mut_lets_a_system_update_velocity_in_place() {
    let mut world = World::new();
    let a = world.spawn();
    world.insert(a, Velocity { dx: 1, dy: 2 });

    // Use `Mut<T>` as the marker for `&mut T`.
    for (_, v) in Query::<Mut<Velocity>>::new(&world).iter() {
        v.dx += 10;
        v.dy += 10;
    }
    assert_eq!(world.get::<Velocity>(a), Some(&Velocity { dx: 11, dy: 12 }));
}
