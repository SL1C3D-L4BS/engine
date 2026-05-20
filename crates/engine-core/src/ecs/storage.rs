//! Component storage backends.
//!
//! Two strategies are provided, matching the spec's hybrid model (IV.3,
//! ADR-002):
//!
//! - [`DenseColumn`] backs `StorageKind::Table` — a slot per entity index,
//!   contiguous and cache-friendly to scan. This is the default hot path.
//! - [`SparseColumn`] backs `StorageKind::SparseSet` — a sparse/dense pair
//!   giving O(1) insert and remove, ideal for tag-like and churn-heavy
//!   components.
//!
//! The archetype-grouped Structure-of-Arrays layout (entities physically
//! grouped by their component set) is the Phase 3 performance rewrite; the
//! foundation layer establishes the correct API and both backends behind it.

use super::StorageKind;
use std::any::Any;

/// Type-erased view of a component column, used to clear a despawned entity's
/// components without knowing their concrete types.
pub(crate) trait AnyColumn: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Removes whatever component is stored for `index`, if any.
    fn erase(&mut self, index: u32);
}

/// `Table` backend: one optional slot per entity index.
pub(crate) struct DenseColumn<T> {
    slots: Vec<Option<T>>,
}

impl<T> DenseColumn<T> {
    fn new() -> Self {
        Self { slots: Vec::new() }
    }

    fn insert(&mut self, index: u32, value: T) {
        let i = index as usize;
        if i >= self.slots.len() {
            self.slots.resize_with(i + 1, || None);
        }
        self.slots[i] = Some(value);
    }

    fn remove(&mut self, index: u32) -> Option<T> {
        self.slots.get_mut(index as usize).and_then(Option::take)
    }

    fn get(&self, index: u32) -> Option<&T> {
        self.slots.get(index as usize).and_then(Option::as_ref)
    }

    fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        self.slots.get_mut(index as usize).and_then(Option::as_mut)
    }
}

/// `SparseSet` backend: a sparse index table over a packed dense array.
pub(crate) struct SparseColumn<T> {
    /// `entity index -> dense position`, or `u32::MAX` when absent.
    sparse: Vec<u32>,
    /// `dense position -> entity index`.
    dense_index: Vec<u32>,
    /// `dense position -> value`.
    dense_value: Vec<T>,
}

const ABSENT: u32 = u32::MAX;

impl<T> SparseColumn<T> {
    fn new() -> Self {
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

    fn insert(&mut self, index: u32, value: T) {
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

    fn remove(&mut self, index: u32) -> Option<T> {
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

    fn get(&self, index: u32) -> Option<&T> {
        self.dense_pos(index).map(|p| &self.dense_value[p])
    }

    fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        match self.dense_pos(index) {
            Some(p) => Some(&mut self.dense_value[p]),
            None => None,
        }
    }
}

/// A component column: one of the two storage backends, chosen per component
/// type from `Component::STORAGE`.
pub(crate) enum ComponentColumn<T> {
    Dense(DenseColumn<T>),
    Sparse(SparseColumn<T>),
}

impl<T> ComponentColumn<T> {
    pub(crate) fn new(kind: StorageKind) -> Self {
        match kind {
            StorageKind::Table => ComponentColumn::Dense(DenseColumn::new()),
            StorageKind::SparseSet => ComponentColumn::Sparse(SparseColumn::new()),
        }
    }

    pub(crate) fn insert(&mut self, index: u32, value: T) {
        match self {
            ComponentColumn::Dense(c) => c.insert(index, value),
            ComponentColumn::Sparse(c) => c.insert(index, value),
        }
    }

    pub(crate) fn remove(&mut self, index: u32) -> Option<T> {
        match self {
            ComponentColumn::Dense(c) => c.remove(index),
            ComponentColumn::Sparse(c) => c.remove(index),
        }
    }

    pub(crate) fn get(&self, index: u32) -> Option<&T> {
        match self {
            ComponentColumn::Dense(c) => c.get(index),
            ComponentColumn::Sparse(c) => c.get(index),
        }
    }

    pub(crate) fn get_mut(&mut self, index: u32) -> Option<&mut T> {
        match self {
            ComponentColumn::Dense(c) => c.get_mut(index),
            ComponentColumn::Sparse(c) => c.get_mut(index),
        }
    }

    pub(crate) fn contains(&self, index: u32) -> bool {
        self.get(index).is_some()
    }

    /// Entity indices holding this component, always in ascending order so
    /// iteration is deterministic (spec IV.2) regardless of backend.
    pub(crate) fn sorted_indices(&self) -> Vec<u32> {
        let mut indices: Vec<u32> = match self {
            ComponentColumn::Dense(c) => (0..c.slots.len() as u32)
                .filter(|&i| c.slots[i as usize].is_some())
                .collect(),
            ComponentColumn::Sparse(c) => c.dense_index.clone(),
        };
        indices.sort_unstable();
        indices
    }
}

impl<T: 'static> AnyColumn for ComponentColumn<T> {
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

    fn round_trip(kind: StorageKind) {
        let mut col: ComponentColumn<i32> = ComponentColumn::new(kind);
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

        // Iteration order is ascending for both backends.
        assert_eq!(col.sorted_indices(), vec![2, 9]);
    }

    #[test]
    fn dense_backend_round_trips() {
        round_trip(StorageKind::Table);
    }

    #[test]
    fn sparse_backend_round_trips() {
        round_trip(StorageKind::SparseSet);
    }
}
