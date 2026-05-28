use serde::{Deserialize, Serialize};

/// Entity identifier with generation for stale-handle protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entity {
    index: u32,
    generation: u32,
}

impl Entity {
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }
}

/// Manages entity creation, destruction, and generation tracking.
///
/// Uses a free-list for recycled indices and increments generations on free
/// to invalidate stale handles.
pub struct EntityManager {
    generations: Vec<u32>,
    free_list: Vec<u32>,
}

impl EntityManager {
    pub fn new() -> Self {
        Self {
            generations: Vec::new(),
            free_list: Vec::new(),
        }
    }

    /// Allocate a new entity handle.
    ///
    /// Returns an [`Entity`] with a unique (index, generation) pair.  If a
    /// previously freed index is available it is recycled; otherwise a new
    /// slot is appended.
    pub fn allocate(&mut self) -> Entity {
        if let Some(index) = self.free_list.pop() {
            let generation = self.generations[index as usize];
            Entity::new(index, generation)
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            Entity::new(index, 0)
        }
    }

    /// Free an entity, incrementing its generation so existing handles become
    /// stale.
    ///
    /// Returns `false` if the entity was already freed (stale handle).
    pub fn free(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let idx = entity.index as usize;
        self.generations[idx] += 1;
        self.free_list.push(entity.index);
        true
    }

    /// Returns `true` if the entity handle is still live.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let idx = entity.index as usize;
        idx < self.generations.len() && self.generations[idx] == entity.generation
    }

    /// Number of live entities.
    pub fn alive_count(&self) -> usize {
        self.generations.len() - self.free_list.len()
    }

    /// Total capacity (including freed slots).
    pub fn capacity(&self) -> u32 {
        self.generations.len() as u32
    }
}

impl Default for EntityManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_allocate_and_recycle() {
        let mut mgr = EntityManager::new();
        let a = mgr.allocate();
        let b = mgr.allocate();
        assert_ne!(a, b);
        assert!(mgr.is_alive(a));
        assert!(mgr.is_alive(b));
        assert_eq!(mgr.alive_count(), 2);

        assert!(mgr.free(a));
        assert!(!mgr.is_alive(a));
        assert_eq!(mgr.alive_count(), 1);

        // Recycling should return a's index with bumped generation.
        let c = mgr.allocate();
        assert_eq!(c.index(), a.index());
        assert_ne!(c.generation(), a.generation());
        assert!(mgr.is_alive(c));
        assert_eq!(mgr.alive_count(), 2);
    }

    #[test]
    fn entity_stale_handle_detected() {
        let mut mgr = EntityManager::new();
        let e = mgr.allocate();
        assert!(mgr.free(e));
        // After free, old handle is stale.
        assert!(!mgr.is_alive(e));
        // Double free returns false.
        assert!(!mgr.free(e));
    }

    #[test]
    fn entity_free_nonexistent_returns_false() {
        let mut mgr = EntityManager::new();
        // An entity with index 0, generation 0 before any allocation is invalid.
        assert!(!mgr.free(Entity::new(0, 0)));
    }

    #[test]
    fn entity_capacity_grows() {
        let mut mgr = EntityManager::new();
        let e1 = mgr.allocate();
        let e2 = mgr.allocate();
        let e3 = mgr.allocate();
        assert!(mgr.capacity() >= 3);
        mgr.free(e2);
        // Capacity should not shrink after free.
        assert!(mgr.capacity() >= 3);
        let _recycled = mgr.allocate();
        assert!(mgr.capacity() >= 3);
        let _ = e1;
        let _ = e3;
    }
}
