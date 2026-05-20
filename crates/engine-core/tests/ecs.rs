//! ECS behaviour oracle: the derived `Component`, both storage backends,
//! entity lifecycle, iteration order, the scheduler, and resources.

use engine_core::{Component, Phase, Schedule, StorageKind, World};

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

#[derive(Component, Debug, PartialEq)]
#[component(storage = "SparseSet")]
struct Selected;

#[test]
fn derive_selects_the_storage_backend() {
    assert_eq!(<Position as Component>::STORAGE, StorageKind::Table);
    assert_eq!(<Selected as Component>::STORAGE, StorageKind::SparseSet);
}

#[test]
fn spawn_insert_get_mutate() {
    let mut world = World::new();
    let e = world.spawn();
    assert!(world.insert(e, Position { x: 1, y: 2 }));
    assert_eq!(world.get::<Position>(e), Some(&Position { x: 1, y: 2 }));

    world.get_mut::<Position>(e).unwrap().x = 9;
    assert_eq!(world.get::<Position>(e).unwrap().x, 9);

    assert!(world.contains::<Position>(e));
    assert!(!world.contains::<Velocity>(e));
    assert_eq!(world.entity_count(), 1);
}

#[test]
fn despawn_clears_components_and_invalidates_handles() {
    let mut world = World::new();
    let e = world.spawn();
    world.insert(e, Position { x: 0, y: 0 });

    assert!(world.despawn(e));
    assert!(!world.is_alive(e));
    assert_eq!(world.get::<Position>(e), None);
    // A stale handle cannot resurrect the entity.
    assert!(!world.insert(e, Position { x: 1, y: 1 }));

    // The recycled index carries no leftover component data.
    let e2 = world.spawn();
    assert_eq!(e2.index(), e.index());
    assert_eq!(world.get::<Position>(e2), None);
}

#[test]
fn for_each_visits_in_ascending_index_order() {
    let mut world = World::new();
    for i in 0..10 {
        let e = world.spawn();
        world.insert(e, Position { x: i, y: 0 });
    }
    let mut seen = Vec::new();
    world.for_each::<Position>(|e, p| seen.push((e.index(), p.x)));
    assert_eq!(seen, (0..10).map(|i| (i as u32, i)).collect::<Vec<_>>());
}

#[test]
fn sparse_storage_iterates_and_mutates() {
    let mut world = World::new();
    let a = world.spawn();
    let b = world.spawn();
    world.insert(a, Selected);
    world.insert(b, Selected);
    assert_eq!(world.count::<Selected>(), 2);

    world.remove::<Selected>(a);
    let remaining = world.entities_with::<Selected>();
    assert_eq!(remaining, vec![b]);
}

#[test]
fn scheduler_runs_a_movement_system() {
    let mut world = World::new();
    let e = world.spawn();
    world.insert(e, Position { x: 0, y: 0 });
    world.insert(e, Velocity { dx: 2, dy: -1 });

    let mut schedule = Schedule::new();
    schedule.add_system(Phase::Update, "movement", |world: &mut World| {
        for entity in world.entities_with::<Velocity>() {
            let v = *world.get::<Velocity>(entity).unwrap();
            if let Some(p) = world.get_mut::<Position>(entity) {
                p.x += v.dx;
                p.y += v.dy;
            }
        }
    });

    schedule.run(&mut world);
    schedule.run(&mut world);
    assert_eq!(world.get::<Position>(e), Some(&Position { x: 4, y: -2 }));
}

#[test]
fn resources_round_trip() {
    let mut world = World::new();
    world.insert_resource(100u32);
    *world.resource_mut::<u32>().unwrap() += 1;
    assert_eq!(world.resource::<u32>(), Some(&101));
    assert_eq!(world.remove_resource::<u32>(), Some(101));
    assert_eq!(world.resource::<u32>(), None);
}
