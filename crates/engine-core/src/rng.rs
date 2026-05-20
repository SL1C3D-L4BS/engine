//! Deterministic random number generation.
//!
//! Every random value is derived by hashing `(seed, frame, channel, counter)`
//! with BLAKE3 — there is no global RNG state (spec IV.2, ADR-013). Two runs
//! with the same seed therefore produce identical values, and independent
//! channels (`"physics"`, `"ai.pathing"`, `"vfx.spawn"`, …) never interfere.

/// Derives one `u64` from the full key. This is the stateless primitive every
/// other function builds on.
pub fn derive_u64(seed: u64, frame: u64, channel: &str, counter: u64) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_le_bytes());
    hasher.update(&frame.to_le_bytes());
    hasher.update(channel.as_bytes());
    // A delimiter so `channel` and `counter` cannot be confused with each
    // other across different channel-name lengths.
    hasher.update(&[0xff]);
    hasher.update(&counter.to_le_bytes());
    let hash = hasher.finalize();
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
}

/// A per-frame random source.
///
/// The RNG is scoped to one `(seed, frame)`; create a fresh one each frame.
/// Each call advances a counter, so successive draws differ while remaining
/// fully reproducible.
#[derive(Clone, Debug)]
pub struct Rng {
    seed: u64,
    frame: u64,
    counter: u64,
}

impl Rng {
    /// Creates an RNG for a given seed and frame number.
    pub fn new(seed: u64, frame: u64) -> Self {
        Self {
            seed,
            frame,
            counter: 0,
        }
    }

    /// Draws a uniformly distributed `u64` on `channel`.
    pub fn next_u64(&mut self, channel: &str) -> u64 {
        let value = derive_u64(self.seed, self.frame, channel, self.counter);
        self.counter += 1;
        value
    }

    /// Draws an `f32` uniformly distributed in `[0, 1)`.
    pub fn next_f32(&mut self, channel: &str) -> f32 {
        // Use the top 24 bits — exactly the `f32` mantissa width.
        (self.next_u64(channel) >> 40) as f32 / (1u32 << 24) as f32
    }

    /// Draws a `bool` with even odds.
    pub fn next_bool(&mut self, channel: &str) -> bool {
        self.next_u64(channel) & 1 == 1
    }

    /// Draws an integer uniformly distributed in `[low, high)`.
    ///
    /// Returns `low` if the range is empty.
    pub fn next_range(&mut self, channel: &str, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        let span = (high - low) as u64;
        low + (self.next_u64(channel) % span) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_yields_same_value() {
        assert_eq!(
            derive_u64(42, 7, "physics", 3),
            derive_u64(42, 7, "physics", 3)
        );
    }

    #[test]
    fn channels_and_counters_are_independent() {
        assert_ne!(
            derive_u64(1, 1, "physics", 0),
            derive_u64(1, 1, "ai.pathing", 0)
        );
        assert_ne!(
            derive_u64(1, 1, "physics", 0),
            derive_u64(1, 1, "physics", 1)
        );
    }

    #[test]
    fn rng_sequence_is_reproducible() {
        let mut a = Rng::new(99, 1);
        let mut b = Rng::new(99, 1);
        for _ in 0..100 {
            assert_eq!(a.next_u64("vfx.spawn"), b.next_u64("vfx.spawn"));
        }
    }

    #[test]
    fn next_f32_stays_in_unit_interval() {
        let mut rng = Rng::new(5, 5);
        for _ in 0..1000 {
            let v = rng.next_f32("test");
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn next_range_stays_in_bounds() {
        let mut rng = Rng::new(5, 5);
        for _ in 0..1000 {
            let v = rng.next_range("test", -10, 10);
            assert!((-10..10).contains(&v));
        }
    }
}
