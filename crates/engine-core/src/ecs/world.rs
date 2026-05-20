//! The [`World`] — the container of all entities, components, and resources.

use super::Component;
use super::entity::{Entity, EntityAllocator};
use super::storage::{AnyColumn, ComponentColumn};
use std::any::{Any, TypeId};
use std::collections::HashMap;

/// The ECS world: entities, their components, and global resources.
#[derive(Default)]
pub struct World {
    entities: EntityAllocator,
    columns: HashMap<TypeId, Box<dyn AnyColumn>>,
    resources: HashMap<TypeId, Box<dyn Any>>,
}

impl World {
    /// Creates an empty world.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns a new, component-less entity.
    pub fn spawn(&mut self) -> Entity {
        self.entities.alloc()
    }

    /// Despawns an entity, removing all of its components.
    ///
    /// Returns `false` if the handle was already stale or dead.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        let index = entity.index();
        for column in self.columns.values_mut() {
            column.erase(index);
        }
        self.entities.free(entity)
    }

    /// Returns `true` if `entity` is currently live.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// The number of live entities.
    pub fn entity_count(&self) -> usize {
        self.entities.live_count()
    }

    /// Attaches (or overwrites) component `T` on `entity`.
    ///
    /// Returns `false` without modifying anything if the handle is stale.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        if !self.entities.is_alive(entity) {
            return false;
        }
        self.column_mut::<T>().insert(entity.index(), value);
        true
    }

    /// Removes component `T` from `entity`, returning it if it was present.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let column = self
            .columns
            .get_mut(&TypeId::of::<T>())?
            .as_any_mut()
            .downcast_mut::<ComponentColumn<T>>()?;
        column.remove(entity.index())
    }

    /// Borrows component `T` of `entity`.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        self.column::<T>()?.get(entity.index())
    }

    /// Mutably borrows component `T` of `entity`.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        if !self.entities.is_alive(entity) {
            return None;
        }
        let index = entity.index();
        self.columns
            .get_mut(&TypeId::of::<T>())?
            .as_any_mut()
            .downcast_mut::<ComponentColumn<T>>()?
            .get_mut(index)
    }

    /// Returns `true` if `entity` has component `T`.
    pub fn contains<T: Component>(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
            && self
                .column::<T>()
                .is_some_and(|c| c.contains(entity.index()))
    }

    /// Visits every `(entity, &T)` pair, in ascending entity-index order.
    pub fn for_each<T: Component>(&self, mut f: impl FnMut(Entity, &T)) {
        let Some(column) = self.column::<T>() else {
            return;
        };
        for index in column.sorted_indices() {
            if let Some(entity) = self.entities.entity_at(index)
                && let Some(value) = column.get(index)
            {
                f(entity, value);
            }
        }
    }

    /// Visits every `(entity, &mut T)` pair, in ascending entity-index order.
    pub fn for_each_mut<T: Component>(&mut self, mut f: impl FnMut(Entity, &mut T)) {
        let entities = &self.entities;
        let Some(boxed) = self.columns.get_mut(&TypeId::of::<T>()) else {
            return;
        };
        let Some(column) = boxed.as_any_mut().downcast_mut::<ComponentColumn<T>>() else {
            return;
        };
        for index in column.sorted_indices() {
            if let Some(entity) = entities.entity_at(index)
                && let Some(value) = column.get_mut(index)
            {
                f(entity, value);
            }
        }
    }

    /// Collects every entity that currently has component `T`.
    pub fn entities_with<T: Component>(&self) -> Vec<Entity> {
        let Some(column) = self.column::<T>() else {
            return Vec::new();
        };
        column
            .sorted_indices()
            .into_iter()
            .filter_map(|i| self.entities.entity_at(i))
            .collect()
    }

    /// The number of entities holding component `T`.
    pub fn count<T: Component>(&self) -> usize {
        self.column::<T>()
            .map(|c| c.sorted_indices().len())
            .unwrap_or(0)
    }

    /// Inserts (or replaces) a global resource.
    pub fn insert_resource<R: 'static>(&mut self, resource: R) {
        self.resources.insert(TypeId::of::<R>(), Box::new(resource));
    }

    /// Borrows a global resource.
    pub fn resource<R: 'static>(&self) -> Option<&R> {
        self.resources
            .get(&TypeId::of::<R>())
            .and_then(|r| r.downcast_ref::<R>())
    }

    /// Mutably borrows a global resource.
    pub fn resource_mut<R: 'static>(&mut self) -> Option<&mut R> {
        self.resources
            .get_mut(&TypeId::of::<R>())
            .and_then(|r| r.downcast_mut::<R>())
    }

    /// Removes and returns a global resource.
    pub fn remove_resource<R: 'static>(&mut self) -> Option<R> {
        self.resources
            .remove(&TypeId::of::<R>())
            .and_then(|r| r.downcast::<R>().ok())
            .map(|b| *b)
    }

    fn column<T: Component>(&self) -> Option<&ComponentColumn<T>> {
        self.columns
            .get(&TypeId::of::<T>())
            .and_then(|c| c.as_any().downcast_ref::<ComponentColumn<T>>())
    }

    fn column_mut<T: Component>(&mut self) -> &mut ComponentColumn<T> {
        self.columns
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(ComponentColumn::<T>::new(T::STORAGE)))
            .as_any_mut()
            .downcast_mut::<ComponentColumn<T>>()
            .expect("column concrete type always matches its TypeId key")
    }
}
