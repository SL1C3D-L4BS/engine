//! Owned Robin Hood hash map (ADR-028).
//!
//! Open-addressed table with backward-shift deletion and Robin Hood probe
//! equalization. Power-of-two capacities and a load-factor cap of 7/8 keep
//! the worst-case probe distance tight; backward-shift deletion preserves
//! the variance bound that gives Robin Hood its tail-latency story.
//!
//! Two hashers ship in the same module, both owned in-tree (R-02):
//!
//! - [`FastHasher`] — multiplicative FxHash-style hasher for the hot lookup
//!   path. Inputs are folded by `rot5; xor; mul` with the FxHash constant
//!   `0x517cc1b727220a95`. Not cross-architecture deterministic: the
//!   `write(&[u8])` path interprets chunks in native-endian order.
//! - [`DeterministicHasher`] — BLAKE3-keyed hasher whose finalized digest is
//!   bit-identical across runs, builds, and architectures. Every `write_uN`
//!   override serializes to little-endian so the kernel-level hash bytes are
//!   the same on x86-64 and aarch64. Pay the cost only where iteration or
//!   probe sequence could feed a cross-arch invariant (ECS resource map).
//!
//! No iteration order is promised, even with the deterministic hasher;
//! capacity-dependent probe positions make a stable order a fragile
//! contract. Use [`std::collections::BTreeMap`] if you need ordered
//! iteration.

use std::borrow::Borrow;
use std::hash::{BuildHasher, Hash, Hasher};
use std::iter::FusedIterator;
use std::mem::{self, MaybeUninit};

/// Smallest non-empty capacity. Power of two so the index is a mask.
const MIN_CAPACITY: usize = 16;

/// Load-factor cap of 7/8. Grow when `(len + 1) * LOAD_DEN > capacity * LOAD_NUM`.
const LOAD_NUM: usize = 7;
/// Denominator of the load-factor cap. See [`LOAD_NUM`].
const LOAD_DEN: usize = 8;

/// One bucket of the table. `hash` is the 32-bit truncation of the full
/// finalized hash (cheap probe-skip on collisions); `dib` is the
/// Distance from Initial Bucket and drives the Robin Hood invariant.
struct Slot<K, V> {
    hash: u32,
    dib: u16,
    occupied: bool,
    key: MaybeUninit<K>,
    val: MaybeUninit<V>,
}

impl<K, V> Slot<K, V> {
    fn empty() -> Self {
        Self {
            hash: 0,
            dib: 0,
            occupied: false,
            key: MaybeUninit::uninit(),
            val: MaybeUninit::uninit(),
        }
    }
}

impl<K, V> Drop for Slot<K, V> {
    fn drop(&mut self) {
        if self.occupied {
            unsafe {
                self.key.assume_init_drop();
                self.val.assume_init_drop();
            }
        }
    }
}

/// An owned Robin Hood hash map.
///
/// Default hasher is [`FastHasher`]. Use
/// [`HashMap::with_hasher`](Self::with_hasher) with
/// [`DeterministicHasher::new()`](DeterministicHasher::new) when iteration
/// or probe order must be cross-arch reproducible.
pub struct HashMap<K, V, S = FastHasher> {
    slots: Box<[Slot<K, V>]>,
    len: usize,
    hasher: S,
}

impl<K, V> HashMap<K, V, FastHasher> {
    /// Creates an empty map with the default [`FastHasher`].
    pub fn new() -> Self {
        Self::with_capacity_and_hasher(0, FastHasher::new())
    }

    /// Creates an empty map with at least `capacity` buckets.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, FastHasher::new())
    }
}

impl<K, V> Default for HashMap<K, V, FastHasher> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: std::fmt::Debug, V: std::fmt::Debug, S> std::fmt::Debug for HashMap<K, V, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K, V, S> HashMap<K, V, S> {
    /// Creates an empty map with a caller-supplied hasher.
    pub fn with_hasher(hasher: S) -> Self {
        Self::with_capacity_and_hasher(0, hasher)
    }

    /// Creates an empty map with at least `capacity` buckets and a
    /// caller-supplied hasher.
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        let cap = if capacity == 0 {
            0
        } else {
            min_pow2_capacity(capacity)
        };
        let slots: Vec<Slot<K, V>> = (0..cap).map(|_| Slot::empty()).collect();
        Self {
            slots: slots.into_boxed_slice(),
            len: 0,
            hasher,
        }
    }

    /// The number of live entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the map has no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bucket count currently allocated. Always a power of two, or zero
    /// for a never-grown map.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Removes every entry but keeps the allocated bucket array.
    pub fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            if s.occupied {
                unsafe {
                    s.key.assume_init_drop();
                    s.val.assume_init_drop();
                }
                s.occupied = false;
                s.hash = 0;
                s.dib = 0;
            }
        }
        self.len = 0;
    }

    /// Iterates `(&K, &V)` pairs in implementation-defined order.
    pub fn iter(&self) -> Iter<'_, K, V> {
        Iter {
            slots: self.slots.iter(),
            remaining: self.len,
        }
    }

    /// Iterates `(&K, &mut V)` pairs in implementation-defined order.
    pub fn iter_mut(&mut self) -> IterMut<'_, K, V> {
        let remaining = self.len;
        IterMut {
            slots: self.slots.iter_mut(),
            remaining,
        }
    }

    /// Iterates `&K` in implementation-defined order.
    pub fn keys(&self) -> Keys<'_, K, V> {
        Keys { iter: self.iter() }
    }

    /// Iterates `&V` in implementation-defined order.
    pub fn values(&self) -> Values<'_, K, V> {
        Values { iter: self.iter() }
    }

    /// Iterates `&mut V` in implementation-defined order.
    pub fn values_mut(&mut self) -> ValuesMut<'_, K, V> {
        ValuesMut {
            iter: self.iter_mut(),
        }
    }

    /// Drains every entry, leaving the map empty. The drain handle owns the
    /// reset: if dropped mid-iteration the remaining entries are still
    /// dropped and the map's state is restored.
    pub fn drain(&mut self) -> Drain<'_, K, V, S> {
        Drain {
            map: self,
            cursor: 0,
        }
    }
}

impl<K: Eq + Hash, V, S: BuildHasher> HashMap<K, V, S> {
    /// Inserts `(key, value)`. If `key` was already present, returns the old
    /// value; otherwise returns `None`.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.reserve_for_one();
        let hash = self.hash_key(&key);
        self.insert_raw(hash, key, value)
    }

    /// Returns the value associated with `key`.
    pub fn get<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        if self.slots.is_empty() {
            return None;
        }
        let hash = self.hash_key(key);
        let mask = self.slots.len() - 1;
        let mut idx = (hash as usize) & mask;
        let mut dib: u16 = 0;
        loop {
            let slot = &self.slots[idx];
            if !slot.occupied {
                return None;
            }
            if slot.dib < dib {
                return None;
            }
            if slot.hash == hash {
                let stored = unsafe { slot.key.assume_init_ref() };
                if stored.borrow() == key {
                    return Some(unsafe { slot.val.assume_init_ref() });
                }
            }
            dib = dib.saturating_add(1);
            idx = (idx + 1) & mask;
        }
    }

    /// Mutable counterpart to [`get`](Self::get).
    pub fn get_mut<Q: ?Sized + Hash + Eq>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        if self.slots.is_empty() {
            return None;
        }
        let hash = self.hash_key(key);
        let mask = self.slots.len() - 1;
        let mut idx = (hash as usize) & mask;
        let mut dib: u16 = 0;
        loop {
            let (occupied, slot_dib, slot_hash) = {
                let slot = &self.slots[idx];
                (slot.occupied, slot.dib, slot.hash)
            };
            if !occupied {
                return None;
            }
            if slot_dib < dib {
                return None;
            }
            if slot_hash == hash {
                let matches = {
                    let stored = unsafe { self.slots[idx].key.assume_init_ref() };
                    stored.borrow() == key
                };
                if matches {
                    return Some(unsafe { self.slots[idx].val.assume_init_mut() });
                }
            }
            dib = dib.saturating_add(1);
            idx = (idx + 1) & mask;
        }
    }

    /// `true` when `key` is present.
    pub fn contains_key<Q: ?Sized + Hash + Eq>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
    {
        self.get(key).is_some()
    }

    /// Removes `key`, returning its value if present.
    pub fn remove<Q: ?Sized + Hash + Eq>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        if self.slots.is_empty() {
            return None;
        }
        let hash = self.hash_key(key);
        let mask = self.slots.len() - 1;
        let mut idx = (hash as usize) & mask;
        let mut dib: u16 = 0;
        let found_idx;
        loop {
            let slot = &self.slots[idx];
            if !slot.occupied {
                return None;
            }
            if slot.dib < dib {
                return None;
            }
            if slot.hash == hash {
                let stored = unsafe { slot.key.assume_init_ref() };
                if stored.borrow() == key {
                    found_idx = idx;
                    break;
                }
            }
            dib = dib.saturating_add(1);
            idx = (idx + 1) & mask;
        }

        // Lift the value out, then walk forward shifting any displaced
        // entries back into the now-vacant slot until we hit either an empty
        // slot or one whose DIB is already zero (it would have nothing to
        // gain from a shift).
        let removed = unsafe { self.slots[found_idx].key.assume_init_read() };
        let value = unsafe { self.slots[found_idx].val.assume_init_read() };
        let _ = removed;
        self.slots[found_idx].occupied = false;
        self.len -= 1;

        let mut prev = found_idx;
        loop {
            let cur = (prev + 1) & mask;
            let (occupied, cur_dib, cur_hash) = {
                let s = &self.slots[cur];
                (s.occupied, s.dib, s.hash)
            };
            if !occupied || cur_dib == 0 {
                self.slots[prev].hash = 0;
                self.slots[prev].dib = 0;
                self.slots[prev].occupied = false;
                break;
            }
            let k = unsafe { self.slots[cur].key.assume_init_read() };
            let v = unsafe { self.slots[cur].val.assume_init_read() };
            self.slots[cur].occupied = false;
            self.slots[prev].hash = cur_hash;
            self.slots[prev].dib = cur_dib - 1;
            self.slots[prev].occupied = true;
            self.slots[prev].key = MaybeUninit::new(k);
            self.slots[prev].val = MaybeUninit::new(v);
            prev = cur;
        }
        Some(value)
    }

    /// Drops every entry for which `f` returns `false`.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        if self.slots.is_empty() {
            return;
        }
        let cap = self.slots.len();
        let placeholder: Vec<Slot<K, V>> = (0..cap).map(|_| Slot::empty()).collect();
        let old_slots = mem::replace(&mut self.slots, placeholder.into_boxed_slice());
        self.len = 0;
        for mut slot in old_slots.into_vec().into_iter() {
            if !slot.occupied {
                continue;
            }
            slot.occupied = false;
            let k = unsafe { slot.key.assume_init_read() };
            let mut v = unsafe { slot.val.assume_init_read() };
            if f(&k, &mut v) {
                let hash = self.hash_key(&k);
                self.insert_raw(hash, k, v);
            }
            // dropping `k` and `v` on the `false` branch is the expected
            // path; the slot itself has occupied=false so its Drop is a
            // no-op.
        }
    }

    fn hash_key<Q: ?Sized + Hash>(&self, key: &Q) -> u32 {
        let v = self.hasher.hash_one(key);
        // XOR-fold to 32 bits to reduce upper-bit dependence; the slot's
        // `occupied` bit is the real "no entry" marker, but folding to 0 is
        // still a wasted probe on collisions so we re-map it.
        let h32 = (v ^ (v >> 32)) as u32;
        if h32 == 0 { 1 } else { h32 }
    }

    fn insert_raw(&mut self, mut hash: u32, mut key: K, mut value: V) -> Option<V> {
        let mask = self.slots.len() - 1;
        let mut idx = (hash as usize) & mask;
        let mut dib: u16 = 0;
        loop {
            let slot = &mut self.slots[idx];
            if !slot.occupied {
                slot.hash = hash;
                slot.dib = dib;
                slot.key = MaybeUninit::new(key);
                slot.val = MaybeUninit::new(value);
                slot.occupied = true;
                self.len += 1;
                return None;
            }
            if slot.hash == hash {
                let matches = {
                    let stored = unsafe { slot.key.assume_init_ref() };
                    stored == &key
                };
                if matches {
                    let old = unsafe { mem::replace(slot.val.assume_init_mut(), value) };
                    return Some(old);
                }
            }
            if slot.dib < dib {
                mem::swap(&mut slot.hash, &mut hash);
                mem::swap(&mut slot.dib, &mut dib);
                let displaced_k = unsafe { slot.key.assume_init_read() };
                let displaced_v = unsafe { slot.val.assume_init_read() };
                slot.key = MaybeUninit::new(key);
                slot.val = MaybeUninit::new(value);
                key = displaced_k;
                value = displaced_v;
            }
            dib = dib
                .checked_add(1)
                .expect("probe distance overflowed u16 — table is pathologically full");
            idx = (idx + 1) & mask;
        }
    }

    fn reserve_for_one(&mut self) {
        if self.slots.is_empty() {
            self.grow_to(MIN_CAPACITY);
            return;
        }
        if (self.len + 1) * LOAD_DEN > self.slots.len() * LOAD_NUM {
            self.grow_to(self.slots.len() * 2);
        }
    }

    fn grow_to(&mut self, new_cap: usize) {
        debug_assert!(new_cap.is_power_of_two());
        let fresh: Vec<Slot<K, V>> = (0..new_cap).map(|_| Slot::empty()).collect();
        let old = mem::replace(&mut self.slots, fresh.into_boxed_slice());
        self.len = 0;
        for mut slot in old.into_vec().into_iter() {
            if !slot.occupied {
                continue;
            }
            let hash = slot.hash;
            slot.occupied = false;
            let k = unsafe { slot.key.assume_init_read() };
            let v = unsafe { slot.val.assume_init_read() };
            self.insert_raw(hash, k, v);
        }
    }
}

#[doc(hidden)]
impl<K, V, S> HashMap<K, V, S> {
    /// Returns a histogram of `dib` values for every occupied slot. Used by
    /// the parity oracle to pin Robin Hood algorithmic behaviour and so is
    /// part of the test contract — not stable API otherwise.
    pub fn probe_distance_histogram(&self) -> Vec<u32> {
        let max = self
            .slots
            .iter()
            .filter(|s| s.occupied)
            .map(|s| s.dib)
            .max()
            .unwrap_or(0);
        let mut hist = vec![0u32; (max as usize) + 1];
        for s in self.slots.iter() {
            if s.occupied {
                hist[s.dib as usize] += 1;
            }
        }
        hist
    }
}

fn min_pow2_capacity(want: usize) -> usize {
    // The table grows when `len * LOAD_DEN > cap * LOAD_NUM`, so to hold
    // `want` entries we need cap >= want * LOAD_DEN / LOAD_NUM, then rounded
    // up to a power of two of at least MIN_CAPACITY.
    let target = want.saturating_mul(LOAD_DEN).div_ceil(LOAD_NUM);
    let mut cap = MIN_CAPACITY;
    while cap < target {
        cap = cap
            .checked_mul(2)
            .expect("requested HashMap capacity exceeds usize");
    }
    cap
}

/// Borrowed iterator over `(K, V)` pairs.
pub struct Iter<'a, K, V> {
    slots: std::slice::Iter<'a, Slot<K, V>>,
    remaining: usize,
}

impl<'a, K, V> Iterator for Iter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        for slot in self.slots.by_ref() {
            if slot.occupied {
                self.remaining -= 1;
                return Some(unsafe { (slot.key.assume_init_ref(), slot.val.assume_init_ref()) });
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, K, V> ExactSizeIterator for Iter<'a, K, V> {}
impl<'a, K, V> FusedIterator for Iter<'a, K, V> {}

/// Mutably-borrowed iterator over `(K, V)` pairs.
pub struct IterMut<'a, K, V> {
    slots: std::slice::IterMut<'a, Slot<K, V>>,
    remaining: usize,
}

impl<'a, K, V> Iterator for IterMut<'a, K, V> {
    type Item = (&'a K, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        for slot in self.slots.by_ref() {
            if slot.occupied {
                self.remaining -= 1;
                let key_ref: &K = unsafe { slot.key.assume_init_ref() };
                let val_mut: &mut V = unsafe { slot.val.assume_init_mut() };
                return Some((key_ref, val_mut));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, K, V> ExactSizeIterator for IterMut<'a, K, V> {}
impl<'a, K, V> FusedIterator for IterMut<'a, K, V> {}

/// Iterator over `&K`.
pub struct Keys<'a, K, V> {
    iter: Iter<'a, K, V>,
}

impl<'a, K, V> Iterator for Keys<'a, K, V> {
    type Item = &'a K;
    fn next(&mut self) -> Option<&'a K> {
        self.iter.next().map(|(k, _)| k)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for Keys<'a, K, V> {}
impl<'a, K, V> FusedIterator for Keys<'a, K, V> {}

/// Iterator over `&V`.
pub struct Values<'a, K, V> {
    iter: Iter<'a, K, V>,
}

impl<'a, K, V> Iterator for Values<'a, K, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<&'a V> {
        self.iter.next().map(|(_, v)| v)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for Values<'a, K, V> {}
impl<'a, K, V> FusedIterator for Values<'a, K, V> {}

/// Iterator over `&mut V`.
pub struct ValuesMut<'a, K, V> {
    iter: IterMut<'a, K, V>,
}

impl<'a, K, V> Iterator for ValuesMut<'a, K, V> {
    type Item = &'a mut V;
    fn next(&mut self) -> Option<&'a mut V> {
        self.iter.next().map(|(_, v)| v)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, K, V> ExactSizeIterator for ValuesMut<'a, K, V> {}
impl<'a, K, V> FusedIterator for ValuesMut<'a, K, V> {}

/// Owning iterator over `(K, V)`.
pub struct IntoIter<K, V> {
    // Walked element-by-element so each slot drops correctly: when this
    // iterator is dropped, any slots we haven't yielded yet drop normally
    // via the Vec's Drop impl, which calls Slot::drop, which drops live
    // keys/values.
    slots: std::vec::IntoIter<Slot<K, V>>,
    remaining: usize,
}

impl<K, V> Iterator for IntoIter<K, V> {
    type Item = (K, V);

    fn next(&mut self) -> Option<(K, V)> {
        for mut slot in self.slots.by_ref() {
            if slot.occupied {
                self.remaining -= 1;
                slot.occupied = false;
                let k = unsafe { slot.key.assume_init_read() };
                let v = unsafe { slot.val.assume_init_read() };
                return Some((k, v));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V> ExactSizeIterator for IntoIter<K, V> {}
impl<K, V> FusedIterator for IntoIter<K, V> {}

impl<K, V, S> IntoIterator for HashMap<K, V, S> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> IntoIter<K, V> {
        let remaining = self.len;
        IntoIter {
            slots: self.slots.into_vec().into_iter(),
            remaining,
        }
    }
}

impl<'a, K, V, S> IntoIterator for &'a HashMap<K, V, S> {
    type Item = (&'a K, &'a V);
    type IntoIter = Iter<'a, K, V>;

    fn into_iter(self) -> Iter<'a, K, V> {
        self.iter()
    }
}

impl<'a, K, V, S> IntoIterator for &'a mut HashMap<K, V, S> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = IterMut<'a, K, V>;

    fn into_iter(self) -> IterMut<'a, K, V> {
        self.iter_mut()
    }
}

/// Draining iterator. On drop, any remaining entries are dropped and the
/// map is left empty.
pub struct Drain<'a, K, V, S> {
    map: &'a mut HashMap<K, V, S>,
    cursor: usize,
}

impl<'a, K, V, S> Iterator for Drain<'a, K, V, S> {
    type Item = (K, V);

    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.map.slots.len() {
            let i = self.cursor;
            self.cursor += 1;
            if self.map.slots[i].occupied {
                self.map.slots[i].occupied = false;
                let k = unsafe { self.map.slots[i].key.assume_init_read() };
                let v = unsafe { self.map.slots[i].val.assume_init_read() };
                self.map.len -= 1;
                return Some((k, v));
            }
        }
        None
    }
}

impl<'a, K, V, S> Drop for Drain<'a, K, V, S> {
    fn drop(&mut self) {
        while self.cursor < self.map.slots.len() {
            let i = self.cursor;
            self.cursor += 1;
            if self.map.slots[i].occupied {
                unsafe {
                    self.map.slots[i].key.assume_init_drop();
                    self.map.slots[i].val.assume_init_drop();
                }
                self.map.slots[i].occupied = false;
            }
        }
        for s in self.map.slots.iter_mut() {
            s.hash = 0;
            s.dib = 0;
        }
        self.map.len = 0;
    }
}

// =============================================================================
// Hashers
// =============================================================================

/// FxHash-style multiplicative hasher. Designed for keys that are already
/// well-distributed (TypeId, content hashes, integer ids).
///
/// Not cross-architecture deterministic: the byte-wise `write` path reads
/// 8-byte chunks in native-endian order. Use [`DeterministicHasher`] when
/// that matters.
#[derive(Clone)]
pub struct FastHasher;

impl FastHasher {
    /// Creates a fresh [`FastHasher`] build state.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FastHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHasher for FastHasher {
    type Hasher = FxHasherImpl;

    fn build_hasher(&self) -> FxHasherImpl {
        FxHasherImpl { hash: 0 }
    }
}

/// One-shot FxHash hasher state.
#[doc(hidden)]
pub struct FxHasherImpl {
    hash: u64,
}

impl FxHasherImpl {
    #[inline(always)]
    fn add(&mut self, x: u64) {
        const FX_MUL: u64 = 0x517c_c1b7_2722_0a95;
        self.hash = (self.hash.rotate_left(5) ^ x).wrapping_mul(FX_MUL);
    }
}

impl Hasher for FxHasherImpl {
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, mut bytes: &[u8]) {
        while bytes.len() >= 8 {
            let chunk: [u8; 8] = bytes[..8].try_into().unwrap();
            self.add(u64::from_ne_bytes(chunk));
            bytes = &bytes[8..];
        }
        if !bytes.is_empty() {
            let mut tail: u64 = 0;
            for (i, &b) in bytes.iter().enumerate() {
                tail |= (b as u64) << (i * 8);
            }
            self.add(tail);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }
    fn write_u16(&mut self, n: u16) {
        self.add(n as u64);
    }
    fn write_u32(&mut self, n: u32) {
        self.add(n as u64);
    }
    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }
    fn write_u128(&mut self, n: u128) {
        self.add(n as u64);
        self.add((n >> 64) as u64);
    }
    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }
    fn write_i8(&mut self, n: i8) {
        self.add(n as u8 as u64);
    }
    fn write_i16(&mut self, n: i16) {
        self.add(n as u16 as u64);
    }
    fn write_i32(&mut self, n: i32) {
        self.add(n as u32 as u64);
    }
    fn write_i64(&mut self, n: i64) {
        self.add(n as u64);
    }
    fn write_isize(&mut self, n: isize) {
        self.add(n as usize as u64);
    }
}

/// BLAKE3-keyed hasher whose output is bit-identical across runs, builds,
/// and architectures. Every `write_uN` / `write_iN` override serializes its
/// operand to little-endian before feeding it to BLAKE3, so two
/// architectures hashing the same [`Hash`]-keyed input produce the same
/// 64-bit digest.
///
/// Fixed 32-byte key — there is no cryptographic privacy story here, and a
/// stable key is exactly what cross-arch determinism requires.
#[derive(Clone)]
pub struct DeterministicHasher;

/// Key used by [`DeterministicHasher`]. 32 bytes of ASCII so the constant is
/// readable in a hex dump; not secret.
const DETERMINISTIC_KEY: [u8; 32] = *b"engine-core-deterministic-hasher";

impl DeterministicHasher {
    /// Creates a fresh [`DeterministicHasher`] build state.
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeterministicHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildHasher for DeterministicHasher {
    type Hasher = DeterministicHasherImpl;

    fn build_hasher(&self) -> DeterministicHasherImpl {
        DeterministicHasherImpl {
            inner: blake3::Hasher::new_keyed(&DETERMINISTIC_KEY),
        }
    }
}

/// One-shot keyed-BLAKE3 hasher state.
#[doc(hidden)]
pub struct DeterministicHasherImpl {
    inner: blake3::Hasher,
}

impl Hasher for DeterministicHasherImpl {
    fn finish(&self) -> u64 {
        let hash = self.inner.finalize();
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
    }

    fn write(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    fn write_u8(&mut self, n: u8) {
        self.inner.update(&[n]);
    }
    fn write_u16(&mut self, n: u16) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_u32(&mut self, n: u32) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_u64(&mut self, n: u64) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_u128(&mut self, n: u128) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_usize(&mut self, n: usize) {
        // Cross-arch determinism requires a width-stable representation.
        self.inner.update(&(n as u64).to_le_bytes());
    }
    fn write_i8(&mut self, n: i8) {
        self.inner.update(&[n as u8]);
    }
    fn write_i16(&mut self, n: i16) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_i32(&mut self, n: i32) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_i64(&mut self, n: i64) {
        self.inner.update(&n.to_le_bytes());
    }
    fn write_isize(&mut self, n: isize) {
        self.inner.update(&(n as i64).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_map_behaviour() {
        let map: HashMap<u32, u32> = HashMap::new();
        assert_eq!(map.len(), 0);
        assert!(map.is_empty());
        assert_eq!(map.capacity(), 0);
        assert_eq!(map.get(&0), None);
        assert!(!map.contains_key(&0));
    }

    #[test]
    fn insert_get_round_trip() {
        let mut map: HashMap<u32, &'static str> = HashMap::new();
        assert_eq!(map.insert(1, "one"), None);
        assert_eq!(map.insert(2, "two"), None);
        assert_eq!(map.insert(3, "three"), None);
        assert_eq!(map.get(&1), Some(&"one"));
        assert_eq!(map.get(&2), Some(&"two"));
        assert_eq!(map.get(&3), Some(&"three"));
        assert_eq!(map.get(&4), None);
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn insert_replaces_existing_value() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        map.insert(1, 10);
        assert_eq!(map.insert(1, 11), Some(10));
        assert_eq!(map.get(&1), Some(&11));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn remove_returns_old_value() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        map.insert(1, 10);
        map.insert(2, 20);
        assert_eq!(map.remove(&1), Some(10));
        assert_eq!(map.get(&1), None);
        assert_eq!(map.get(&2), Some(&20));
        assert_eq!(map.len(), 1);
        assert_eq!(map.remove(&1), None);
    }

    #[test]
    fn grow_preserves_entries() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        for i in 0..1024u32 {
            map.insert(i, i.wrapping_mul(31));
        }
        assert_eq!(map.len(), 1024);
        for i in 0..1024u32 {
            assert_eq!(map.get(&i), Some(&i.wrapping_mul(31)));
        }
    }

    #[test]
    fn iteration_visits_every_entry_once() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        for i in 0..100u32 {
            map.insert(i, i * 2);
        }
        let mut seen: Vec<(u32, u32)> = map.iter().map(|(k, v)| (*k, *v)).collect();
        seen.sort_unstable();
        let expected: Vec<(u32, u32)> = (0..100u32).map(|i| (i, i * 2)).collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn drain_empties_the_map() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        for i in 0..50u32 {
            map.insert(i, i);
        }
        let drained: Vec<(u32, u32)> = map.drain().collect();
        assert_eq!(drained.len(), 50);
        assert_eq!(map.len(), 0);
        assert_eq!(map.get(&5), None);
        // Reinsert is fine after drain — slots are reset.
        map.insert(42, 42);
        assert_eq!(map.get(&42), Some(&42));
    }

    #[test]
    fn drop_drops_owned_values() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let counter = Arc::new(AtomicUsize::new(0));
        struct Dropper(Arc<AtomicUsize>);
        impl Drop for Dropper {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        {
            let mut map: HashMap<u32, Dropper> = HashMap::new();
            for i in 0..200u32 {
                map.insert(i, Dropper(Arc::clone(&counter)));
            }
            // Replace half of them — old droppers must run.
            for i in 0..100u32 {
                map.insert(i, Dropper(Arc::clone(&counter)));
            }
            assert_eq!(counter.load(Ordering::SeqCst), 100);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 300);
    }

    #[test]
    fn deterministic_hasher_is_cross_run_stable() {
        let mut a: HashMap<u64, u64, DeterministicHasher> =
            HashMap::with_hasher(DeterministicHasher::new());
        let mut b: HashMap<u64, u64, DeterministicHasher> =
            HashMap::with_hasher(DeterministicHasher::new());
        for i in 0..1024u64 {
            a.insert(i, i);
            b.insert(i, i);
        }
        assert_eq!(a.probe_distance_histogram(), b.probe_distance_histogram());
    }

    #[test]
    fn retain_keeps_only_matching_entries() {
        let mut map: HashMap<u32, u32> = HashMap::new();
        for i in 0..32u32 {
            map.insert(i, i);
        }
        map.retain(|k, _| *k % 2 == 0);
        assert_eq!(map.len(), 16);
        for i in 0..32u32 {
            if i % 2 == 0 {
                assert_eq!(map.get(&i), Some(&i));
            } else {
                assert_eq!(map.get(&i), None);
            }
        }
    }

    #[test]
    fn min_pow2_capacity_rounding() {
        // The load factor cap is 7/8, so cap >= ceil(want * 8 / 7) then
        // rounded up to a power of two ≥ MIN_CAPACITY.
        assert_eq!(min_pow2_capacity(1), MIN_CAPACITY);
        assert_eq!(min_pow2_capacity(14), MIN_CAPACITY); // ceil(112/7) = 16
        assert_eq!(min_pow2_capacity(15), 32); // ceil(120/7) = 18 → 32
        assert_eq!(min_pow2_capacity(MIN_CAPACITY), 32); // ceil(128/7) = 19 → 32
        assert_eq!(min_pow2_capacity(100), 128); // ceil(800/7) = 115 → 128
        assert_eq!(min_pow2_capacity(1000), 2048); // ceil(8000/7) = 1143 → 2048
    }
}
