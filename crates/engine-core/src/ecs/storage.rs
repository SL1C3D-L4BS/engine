//! Sparse component storage — the SparseSet backend (spec IV.3, ADR-002).
//!
//! Table components live in archetype-grouped columns (see
//! [`super::archetype`]). SparseSet components are kept out of the archetype
//! signature: they're churn-heavy or tag-like by design, so paying the
//! migration cost of an archetype move on every insert/remove would defeat
//! the whole point of the backend. Instead each SparseSet component type
//! owns a single world-scoped [`SparseColumn<T>`], indexed by the entity's
//! storage slot, and queries that mix Table and Sparse components join via
//! sparse lookup.
//!
//! ADR-002 calls the two backends "complement, not alternative"; the
//! archetype redesign in ADR-031 preserves that.

use std::any::Any;

/// Type-erased handle to a SparseSet column, used to clear a despawned
/// entity's components without knowing their concrete type.
pub(crate) trait AnySparseColumn: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Removes whatever component is stored for `index`, if any.
    fn erase(&mut self, index: u32);
}

const ABSENT: u32 = u32::MAX;

/// A sparse index table over a packed dense array — O(1) insert, remove,
/// and lookup with no per-entity wastage when the population is sparse.
pub(crate) struct SparseColumn<T> {
    /// `entity index -> dense position`, or [`ABSENT`] when absent.
    sparse: Vec<u32>,
    /// `dense position -> entity index`.
    dense_index: Vec<u32>,
    /// `dense position -> value`.
    dense_value: Vec<T>,
}

impl<T> SparseColumn<T> {
    pub(crate) fn new() -> Self {
        Self {
            sparse: Vec::new(),
            dense_index: Vec::new(),
            dense_value: Vec::new(),
        }
    }

    fn dense_pos(&self, index: u32) -> Option<usize> {
        match self.sparse.get(index as usize).copied() {
            Some(p) if p != ABSENT => Some(p as usize),
            _ => None,
        }
    }

    pub(crate) fn insert(&mut self, index: u32, value: T) {
        let i = index as usize;
        if i >= self.sparse.len() {
            self.sparse.resize(i + 1, ABSENT);
        }
        if let Some(pos) = self.dense_pos(index) {
            self.dense_value[pos] = value;
        } else {
            self.sparse[i] = self.dense_index.len() as u32;
            self.dense_index.push(index);
            self.dense_value.push(value);
        }
    }

    pub(crate) fn remove(&mut self, index: u32) -> Option<T> {
        let pos = self.dense_pos(index)?;
        let last = self.dense_index.len() - 1;
        self.dense_index.swap(pos, last);
        self.dense_value.swap(pos, last);
        let moved_index = self.dense_index[pos];
        self.sparse[moved_index as usize] = pos as u32;
        self.sparse[index as usize] = ABSENT;
        self.dense_index.pop();
        self.dense_value.pop()
    }

    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        self.dense_pos(index).map(|p| &self.dense_value[p])
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        match self.dense_pos(index) {
            Some(p) => Some(&mut self.dense_value[p]),
            None => None,
        }
    }

    pub(crate) fn contains(&self, index: u32) -> bool {
        self.dense_pos(index).is_some()
    }

    /// Entity indices holding this component, always in ascending order so
    /// iteration is deterministic (spec IV.2) regardless of insert history.
    pub(crate) fn sorted_indices(&self) -> Vec<u32> {
        let mut indices = self.dense_index.clone();
        indices.sort_unstable();
        indices
    }
}

impl<T: 'static> AnySparseColumn for SparseColumn<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn erase(&mut self, index: u32) {
        self.remove(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_round_trips() {
        let mut col: SparseColumn<i32> = SparseColumn::new();
        col.insert(5, 50);
        col.insert(2, 20);
        col.insert(9, 90);
        assert_eq!(col.get(5), Some(&50));
        assert!(col.contains(2));
        assert!(!col.contains(3));

        *col.get_mut(2).unwrap() = 22;
        assert_eq!(col.get(2), Some(&22));

        assert_eq!(col.remove(5), Some(50));
        assert!(!col.contains(5));
        assert_eq!(col.remove(5), None);

        // Iteration order is ascending.
        assert_eq!(col.sorted_indices(), vec![2, 9]);
    }
}
