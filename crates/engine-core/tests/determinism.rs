//! Cross-architecture determinism oracle for `engine-core`.
//!
//! Exercises the BLAKE3-keyed RNG and a scripted ECS build, reduces the result
//! to an FNV-1a digest, and asserts it against a committed golden. Both CI
//! architectures compare to the same golden, so passing runs are byte-equal
//! to each other (spec IV.2, ADR-013).
//!
//! Regenerate after an intentional change:
//! `ENGINE_GOLDEN_WRITE=1 cargo test -p engine-core --test determinism`.

use engine_core::Component;
use engine_core::rng::{Rng, derive_u64};

#[derive(Component)]
struct Health {
    current: i32,
    max: i32,
}

#[derive(Component)]
#[component(storage = "SparseSet")]
struct Tag {
    value: u64,
}

// --- Phase 3 archetype sweep components (ADR-031) -------------------------
// Six small Table components plus two SparseSet components. The sweep below
// constructs 64 distinct archetype signatures by picking subsets of the six
// Table components, exercises insertion edges across them, and folds the
// resulting `World::query` output into the digest.

#[derive(Component)]
struct ArchA {
    a: i32,
}

#[derive(Component)]
struct ArchB {
    b: i32,
}

#[derive(Component)]
struct ArchC {
    c: i32,
}

#[derive(Component)]
struct ArchD {
    d: i32,
}

#[derive(Component)]
struct ArchE {
    e: i32,
}

#[derive(Component)]
struct ArchF {
    f: i32,
}

#[derive(Component)]
#[component(storage = "SparseSet")]
struct ArchTag {
    t: u32,
}

#[derive(Component)]
#[component(storage = "SparseSet")]
struct ArchMark {
    m: u32,
}

struct Digest {
    hash: u64,
}

impl Digest {
    fn new() -> Self {
        Self {
            hash: 0xcbf2_9ce4_8422_2325,
        }
    }

    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.hash ^= b as u64;
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn i64(&mut self, v: i64) {
        self.u64(v as u64);
    }
}

fn compute() -> u64 {
    let mut d = Digest::new();

    // --- RNG: the stateless primitive across frames and channels ---------
    for frame in 0..64 {
        for counter in 0..16 {
            d.u64(derive_u64(0xDEAD_BEEF, frame, "physics", counter));
            d.u64(derive_u64(0xDEAD_BEEF, frame, "ai.pathing", counter));
        }
    }

    // --- RNG: the stateful sequence -------------------------------------
    let mut rng = Rng::new(0x1234_5678, 7);
    for _ in 0..512 {
        d.u64(rng.next_u64("vfx.spawn"));
    }
    for _ in 0..512 {
        d.u64(rng.next_f32("vfx.jitter").to_bits() as u64);
    }

    // --- ECS: a scripted world build, hashed via a canonical snapshot ----
    let mut world = engine_core::World::new();
    let mut entities = Vec::new();
    for i in 0..256 {
        let e = world.spawn();
        world.insert(
            e,
            Health {
                current: i,
                max: 100 + i,
            },
        );
        if i % 3 == 0 {
            world.insert(
                e,
                Tag {
                    value: i as u64 * 7,
                },
            );
        }
        entities.push(e);
    }
    for i in (0..256).step_by(5) {
        world.despawn(entities[i]);
    }
    // Snapshot: each storage backend iterated in deterministic index order.
    world.for_each::<Health>(|e, h| {
        d.u64(e.to_bits());
        d.i64(h.current as i64);
        d.i64(h.max as i64);
    });
    world.for_each::<Tag>(|e, t| {
        d.u64(e.to_bits());
        d.u64(t.value);
    });

    // --- ECS: 64 distinct archetypes, mixed Table + SparseSet (ADR-031) -
    // Each entity's Table-component set is the lower 6 bits of its index;
    // SparseSet components are sprinkled by separate masks. 192 entities
    // populate all 64 signatures (1+1+1+1 = at least three per signature),
    // exercising adjacency edges and swap-remove correctness.
    let mut arch_world = engine_core::World::new();
    let mut arch_entities = Vec::new();
    for i in 0..192i32 {
        let e = arch_world.spawn();
        let mask = (i as u32) & 0x3F; // 6 bits → 64 archetypes
        if mask & 0b000001 != 0 {
            arch_world.insert(e, ArchA { a: i });
        }
        if mask & 0b000010 != 0 {
            arch_world.insert(e, ArchB { b: i + 1 });
        }
        if mask & 0b000100 != 0 {
            arch_world.insert(e, ArchC { c: i + 2 });
        }
        if mask & 0b001000 != 0 {
            arch_world.insert(e, ArchD { d: i + 3 });
        }
        if mask & 0b010000 != 0 {
            arch_world.insert(e, ArchE { e: i + 4 });
        }
        if mask & 0b100000 != 0 {
            arch_world.insert(e, ArchF { f: i + 5 });
        }
        if (i as u32).is_multiple_of(5) {
            arch_world.insert(e, ArchTag { t: i as u32 });
        }
        if (i as u32).is_multiple_of(7) {
            arch_world.insert(e, ArchMark { m: (i * 13) as u32 });
        }
        arch_entities.push(e);
    }
    // Despawn every fourth entity to exercise archetype swap-remove paths.
    for i in (0..arch_entities.len()).step_by(4) {
        arch_world.despawn(arch_entities[i]);
    }

    // Fold the typed query output into the digest. Iteration order is
    // ascending ArchetypeId then ascending row; both halves are fixed by
    // the deterministic archetype interning hasher, so the resulting byte
    // sequence is reproducible across runs and architectures.
    arch_world.for_each::<ArchA>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.a as i64);
    });
    arch_world.for_each::<ArchB>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.b as i64);
    });
    arch_world.for_each::<ArchC>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.c as i64);
    });
    arch_world.for_each::<ArchD>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.d as i64);
    });
    arch_world.for_each::<ArchE>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.e as i64);
    });
    arch_world.for_each::<ArchF>(|e, c| {
        d.u64(e.to_bits());
        d.i64(c.f as i64);
    });
    arch_world.for_each::<ArchTag>(|e, c| {
        d.u64(e.to_bits());
        d.u64(c.t as u64);
    });
    arch_world.for_each::<ArchMark>(|e, c| {
        d.u64(e.to_bits());
        d.u64(c.m as u64);
    });

    // Joint query: (&A, &B) — runs through the archetype-walking
    // QueryIter, fixing iteration order at the ArchetypeId level.
    for (e, a, b) in arch_world.query::<(&ArchA, &ArchB)>().iter() {
        d.u64(e.to_bits());
        d.i64(a.a as i64);
        d.i64(b.b as i64);
    }

    d.hash
}

#[test]
fn core_is_byte_identical_to_golden() {
    let digest = compute();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden-core.txt");

    if std::env::var_os("ENGINE_GOLDEN_WRITE").is_some() {
        std::fs::write(path, format!("{digest:016x}\n")).expect("write golden file");
        eprintln!("wrote golden-core.txt: {digest:016x}");
        return;
    }

    let golden = std::fs::read_to_string(path)
        .expect("tests/golden-core.txt missing — run `just gen-golden`");
    let golden = u64::from_str_radix(golden.trim(), 16).expect("parse golden digest");
    assert_eq!(
        digest, golden,
        "engine-core determinism digest changed: {digest:016x} != golden {golden:016x}"
    );
}

#[test]
fn digest_is_stable_within_a_run() {
    assert_eq!(compute(), compute());
}
