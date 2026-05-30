use crate::Entity;

/// Marker trait for ECS components.
///
/// Each component type must provide a unique [`TYPE_ID`](Component::TYPE_ID)
/// string that is used for dynamic storage lookups.
pub trait Component: Sized + Send + 'static {
    /// Unique type identifier (e.g. `"engine.transform"`).
    const TYPE_ID: &'static str;
}

// ---------------------------------------------------------------------------
// Sparse-set storage
// ---------------------------------------------------------------------------

/// Sparse-set storage for a single component type.
///
/// Provides O(1) insert, remove, and lookup by entity index.
/// Uses a dense vector of `(entity_index, component)` pairs and a sparse
/// vector mapping entity indices to dense indices.
pub struct SparseSet<T: Component> {
    dense: Vec<(u32, T)>,
    sparse: Vec<Option<u32>>,
}

impl<T: Component> SparseSet<T> {
    pub fn new() -> Self {
        Self {
            dense: Vec::new(),
            sparse: Vec::new(),
        }
    }

    /// Insert a component for the given entity.
    ///
    /// If the entity already has a component, it is overwritten.
    pub fn insert(&mut self, entity: Entity, component: T) {
        let entity_index = entity.index();
        self.ensure_sparse(entity_index);

        if let Some(dense_idx) = self.sparse[entity_index as usize] {
            // Overwrite existing entry.
            self.dense[dense_idx as usize] = (entity_index, component);
        } else {
            let dense_idx = self.dense.len() as u32;
            self.sparse[entity_index as usize] = Some(dense_idx);
            self.dense.push((entity_index, component));
        }
    }

    /// Remove the component for the given entity and return it.
    ///
    /// Returns `None` if the entity does not have this component.
    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let entity_index = entity.index();
        if (entity_index as usize) >= self.sparse.len() {
            return None;
        }

        let dense_idx = self.sparse[entity_index as usize]?;
        let last = self.dense.len() - 1;

        // Unset sparse entry for the entity being removed.
        self.sparse[entity_index as usize] = None;

        if (dense_idx as usize) != last {
            // Swap-remove: pop the last entry and move it into the vacated slot.
            let last_entry = self.dense.pop()?;
            // Update sparse entry for the moved entity.
            self.sparse[last_entry.0 as usize] = Some(dense_idx);
            let old_entry = std::mem::replace(&mut self.dense[dense_idx as usize], last_entry);
            Some(old_entry.1)
        } else {
            let entry = self.dense.pop()?;
            Some(entry.1)
        }
    }

    /// Borrow the component for the given entity.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        let entity_index = entity.index();
        if (entity_index as usize) >= self.sparse.len() {
            return None;
        }
        let dense_idx = self.sparse[entity_index as usize]?;
        Some(&self.dense[dense_idx as usize].1)
    }

    /// Mutably borrow the component for the given entity.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        let entity_index = entity.index();
        if (entity_index as usize) >= self.sparse.len() {
            return None;
        }
        let dense_idx = self.sparse[entity_index as usize]?;
        Some(&mut self.dense[dense_idx as usize].1)
    }

    /// Returns `true` if the entity has this component.
    pub fn contains(&self, entity: Entity) -> bool {
        let entity_index = entity.index();
        (entity_index as usize) < self.sparse.len() && self.sparse[entity_index as usize].is_some()
    }

    /// Iterate over all `(Entity, &T)` pairs in arbitrary order.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> + '_ {
        self.dense.iter().map(|(idx, comp)| {
            // We don't store generations in the sparse set, so we return
            // Entity with generation 0.  This is fine for iteration; world
            // queries should validate lifetime externally.
            (Entity::new(*idx, 0), comp)
        })
    }

    /// Iterate over all `(Entity, &mut T)` pairs in arbitrary order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> + '_ {
        self.dense
            .iter_mut()
            .map(|(idx, comp)| (Entity::new(*idx, 0), comp))
    }

    /// Remove all components from this storage.
    pub fn clear(&mut self) {
        self.dense.clear();
        self.sparse.clear();
    }

    /// Number of component entries.
    pub fn len(&self) -> usize {
        self.dense.len()
    }

    /// Returns `true` if the storage is empty.
    pub fn is_empty(&self) -> bool {
        self.dense.is_empty()
    }

    fn ensure_sparse(&mut self, entity_index: u32) {
        let needed = (entity_index + 1) as usize;
        if self.sparse.len() < needed {
            self.sparse.resize(needed, None);
        }
    }
}

impl<T: Component> Default for SparseSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Dynamic storage trait (for heterogeneous storage in World)
// ---------------------------------------------------------------------------

/// Type-erased component storage for dynamic dispatch in [`World`].
///
/// Intended for future Gate 9 extensions (dynamic component types).
pub trait ComponentStorageDyn: Send {
    /// The `TYPE_ID` of the component type stored in this storage.
    fn type_id(&self) -> &'static str;

    /// Remove the component for the given entity (if present).
    fn remove(&mut self, entity: Entity);

    /// Returns `true` if the entity has a component in this storage.
    fn contains(&self, entity: Entity) -> bool;

    /// Remove all components from this storage.
    fn clear(&mut self);

    /// Number of component entries.
    fn len(&self) -> usize;

    /// Returns `true` if the storage is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Downcast to `&dyn std::any::Any` for typed access.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Downcast to `&mut dyn std::any::Any` for typed access.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Borrow a component as `&dyn Any` by entity.
    fn get_any(&self, entity: Entity) -> Option<&dyn std::any::Any>;

    /// Iterate all `(Entity, &dyn Any)` component references.
    fn iter_any(&self) -> Vec<(Entity, &dyn std::any::Any)>;

    /// Insert a boxed (type-erased) component.
    fn insert_any(&mut self, entity: Entity, component: Box<dyn std::any::Any>)
        -> Result<(), Box<dyn std::any::Any>>;
}

impl<T: Component> ComponentStorageDyn for SparseSet<T> {
    fn type_id(&self) -> &'static str {
        T::TYPE_ID
    }

    fn remove(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn contains(&self, entity: Entity) -> bool {
        self.contains(entity)
    }

    fn clear(&mut self) {
        self.clear();
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn get_any(&self, entity: Entity) -> Option<&dyn std::any::Any> {
        self.get(entity).map(|c| c as &dyn std::any::Any)
    }

    fn iter_any(&self) -> Vec<(Entity, &dyn std::any::Any)> {
        self.dense
            .iter()
            .map(|(idx, comp)| (Entity::new(*idx, 0), comp as &dyn std::any::Any))
            .collect()
    }

    fn insert_any(&mut self, entity: Entity, component: Box<dyn std::any::Any>)
        -> Result<(), Box<dyn std::any::Any>>
    {
        match component.downcast::<T>() {
            Ok(c) => {
                self.insert(entity, *c);
                Ok(())
            }
            Err(c) => Err(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestComp(u32);
    impl Component for TestComp {
        const TYPE_ID: &'static str = "test.test_comp";
    }

    fn e(index: u32) -> Entity {
        Entity::new(index, 0)
    }

    #[test]
    fn sparse_set_insert_and_get() {
        let mut set = SparseSet::<TestComp>::new();
        let e0 = e(0);
        let e1 = e(1);

        set.insert(e0, TestComp(10));
        set.insert(e1, TestComp(20));

        assert_eq!(set.get(e0).map(|c| c.0), Some(10));
        assert_eq!(set.get(e1).map(|c| c.0), Some(20));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn sparse_set_overwrite() {
        let mut set = SparseSet::<TestComp>::new();
        let e0 = e(0);
        set.insert(e0, TestComp(1));
        set.insert(e0, TestComp(2));
        assert_eq!(set.get(e0).map(|c| c.0), Some(2));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn sparse_set_remove() {
        let mut set = SparseSet::<TestComp>::new();
        let e0 = e(0);
        let e1 = e(1);
        set.insert(e0, TestComp(10));
        set.insert(e1, TestComp(20));

        assert_eq!(set.remove(e0).unwrap().0, 10);
        assert!(!set.contains(e0));
        assert!(set.contains(e1));
        assert_eq!(set.len(), 1);

        // Remove non-existent returns None.
        assert!(set.remove(e0).is_none());
    }

    #[test]
    fn sparse_set_contains() {
        let mut set = SparseSet::<TestComp>::new();
        let e0 = e(0);
        assert!(!set.contains(e0));
        set.insert(e0, TestComp(1));
        assert!(set.contains(e0));
    }

    #[test]
    fn sparse_set_clear() {
        let mut set = SparseSet::<TestComp>::new();
        set.insert(e(0), TestComp(1));
        set.insert(e(1), TestComp(2));
        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn sparse_set_iter() {
        let mut set = SparseSet::<TestComp>::new();
        set.insert(e(2), TestComp(30));
        set.insert(e(5), TestComp(60));
        let pairs: Vec<_> = set.iter().collect();
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn sparse_set_get_mut() {
        let mut set = SparseSet::<TestComp>::new();
        let e0 = e(0);
        set.insert(e0, TestComp(1));
        if let Some(c) = set.get_mut(e0) {
            c.0 = 42;
        }
        assert_eq!(set.get(e0).unwrap().0, 42);
    }
}
