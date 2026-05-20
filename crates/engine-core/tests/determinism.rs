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
