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
    alive: Vec<bool>,
    free_list: Vec<u32>,
    alive_count: usize,
    initial_generation: u32,
}

impl EntityManager {
    pub fn new() -> Self {
        Self::with_initial_generation(0)
    }

    /// Create an empty manager whose newly allocated slots start at a
    /// caller-provided generation.
    ///
    /// Worlds use distinct seeds so replacing or clearing a World does not
    /// immediately make handles from the previous World valid again.
    pub(crate) fn with_initial_generation(initial_generation: u32) -> Self {
        Self {
            generations: Vec::new(),
            alive: Vec::new(),
            free_list: Vec::new(),
            alive_count: 0,
            initial_generation,
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
            self.alive[index as usize] = true;
            self.alive_count += 1;
            Entity::new(index, generation)
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(self.initial_generation);
            self.alive.push(true);
            self.alive_count += 1;
            Entity::new(index, self.initial_generation)
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
        self.alive[idx] = false;
        self.generations[idx] = self.generations[idx].wrapping_add(1);
        self.free_list.push(entity.index);
        self.alive_count -= 1;
        true
    }

    /// Returns `true` if the entity handle is still live.
    pub fn is_alive(&self, entity: Entity) -> bool {
        let idx = entity.index as usize;
        idx < self.generations.len()
            && self.alive[idx]
            && self.generations[idx] == entity.generation
    }

    /// Number of live entities.
    pub fn alive_count(&self) -> usize {
        self.alive_count
    }

    /// Return the current live handle for `index`.
    ///
    /// This is useful for index-based world metadata tables: callers must not
    /// guess generation zero because a recycled slot carries a newer
    /// generation.
    pub fn live_entity_at(&self, index: u32) -> Option<Entity> {
        let idx = index as usize;
        if idx >= self.generations.len() || !self.alive[idx] {
            return None;
        }
        Some(Entity::new(index, self.generations[idx]))
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
    fn freed_slot_is_not_alive_before_it_is_reallocated() {
        let mut mgr = EntityManager::new();
        let old = mgr.allocate();
        assert!(mgr.free(old));

        let guessed_next_generation = Entity::new(old.index(), old.generation() + 1);
        assert!(!mgr.is_alive(guessed_next_generation));
        assert!(!mgr.free(guessed_next_generation));

        let recycled = mgr.allocate();
        assert_eq!(recycled, guessed_next_generation);
        assert!(mgr.is_alive(recycled));
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

    #[test]
    fn live_entity_at_returns_the_recycled_generation() {
        let mut mgr = EntityManager::new();
        let first = mgr.allocate();
        assert_eq!(mgr.live_entity_at(first.index()), Some(first));

        assert!(mgr.free(first));
        assert_eq!(mgr.live_entity_at(first.index()), None);

        let recycled = mgr.allocate();
        assert_ne!(recycled.generation(), first.generation());
        assert_eq!(mgr.live_entity_at(recycled.index()), Some(recycled));
    }
}
