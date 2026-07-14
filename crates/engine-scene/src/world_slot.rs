use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::World;

type WorldCell = Mutex<Option<World>>;

/// Stable, shared storage for an optional ECS [`World`].
///
/// Both native engine code and the FFI bridge access the world through this
/// slot, so replacing or clearing a world is serialised with every active
/// reader or writer. Cloning or moving a slot never moves the contained world.
#[derive(Clone, Default)]
pub struct WorldSlot {
    inner: Arc<WorldCell>,
}

impl WorldSlot {
    /// Create an empty world slot.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the current world and return the previous one, if any.
    pub fn replace(&self, world: World) -> Option<World> {
        self.lock().replace(world)
    }

    /// Remove and return the current world, if any.
    pub fn clear(&self) -> Option<World> {
        self.lock().take()
    }

    /// Returns `true` when the slot currently contains a world.
    pub fn has_world(&self) -> bool {
        self.lock().is_some()
    }

    /// Run a closure with shared access to the current world.
    ///
    /// Returns `None` when the slot is empty.
    pub fn with_world<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&World) -> R,
    {
        let guard = self.lock();
        guard.as_ref().map(f)
    }

    /// Run a closure with exclusive access to the current world.
    ///
    /// Returns `None` when the slot is empty.
    pub fn with_world_mut<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut World) -> R,
    {
        let mut guard = self.lock();
        guard.as_mut().map(f)
    }

    /// Create a non-owning reference suitable for global registries.
    pub fn downgrade(&self) -> WeakWorldSlot {
        WeakWorldSlot {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Returns `true` when two slots refer to the same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn lock(&self) -> MutexGuard<'_, Option<World>> {
        // A poisoned world is still owned by this slot. Recovering the guard
        // keeps lifecycle operations safe and lets higher layers report their
        // own sentinel/diagnostic instead of stranding a live allocation.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Non-owning counterpart of [`WorldSlot`].
#[derive(Clone, Default)]
pub struct WeakWorldSlot {
    inner: Weak<WorldCell>,
}

impl WeakWorldSlot {
    /// Upgrade this weak reference while its owning runtime is still alive.
    pub fn upgrade(&self) -> Option<WorldSlot> {
        self.inner.upgrade().map(|inner| WorldSlot { inner })
    }

    /// Returns `true` when two weak references target the same slot.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.inner, &other.inner)
    }

    /// Returns `true` when this weak reference targets `slot`.
    pub fn ptr_eq_slot(&self, slot: &WorldSlot) -> bool {
        Weak::ptr_eq(&self.inner, &Arc::downgrade(&slot.inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_clear_and_access_world() {
        let slot = WorldSlot::new();
        assert!(!slot.has_world());

        let mut world = World::new();
        world.create_entity();
        assert!(slot.replace(world).is_none());
        assert!(slot.has_world());
        assert_eq!(slot.with_world(World::alive_count), Some(1));

        slot.with_world_mut(|world| {
            world.create_entity();
        });
        assert_eq!(slot.with_world(World::alive_count), Some(2));

        let previous = slot.replace(World::new()).expect("previous world");
        assert_eq!(previous.alive_count(), 2);
        assert_eq!(slot.with_world(World::alive_count), Some(0));

        assert!(slot.clear().is_some());
        assert!(!slot.has_world());
        assert_eq!(slot.with_world(World::alive_count), None);
    }

    #[test]
    fn weak_slot_does_not_keep_world_alive() {
        let weak = {
            let slot = WorldSlot::new();
            let weak = slot.downgrade();
            assert!(weak.upgrade().is_some());
            weak
        };

        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn pointer_identity_distinguishes_slots_and_clones() {
        let slot = WorldSlot::new();
        let clone = slot.clone();
        let other = WorldSlot::new();
        let weak = slot.downgrade();

        assert!(slot.ptr_eq(&clone));
        assert!(!slot.ptr_eq(&other));
        assert!(weak.ptr_eq_slot(&slot));
        assert!(weak.ptr_eq(&clone.downgrade()));
        assert!(!weak.ptr_eq_slot(&other));
    }
}
