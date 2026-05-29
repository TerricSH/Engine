use crate::Entity;

/// The kind of collision event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionEventKind {
    /// Two colliders started touching.
    ContactStarted,
    /// Two colliders stopped touching.
    ContactStopped,
    /// Touch (persistent contact) — currently unused but reserved.
    Touch,
}

/// A single collision event produced during a physics step.
#[derive(Clone, Debug)]
pub struct CollisionEvent {
    /// What kind of event this is.
    pub kind: CollisionEventKind,
    /// The first entity involved.
    pub entity_a: Entity,
    /// The second entity involved.
    pub entity_b: Entity,
}

/// Collection of collision events from a physics step.
#[derive(Clone, Debug, Default)]
pub struct PhysicsEvents {
    pub events: Vec<CollisionEvent>,
}

impl PhysicsEvents {
    /// Create a new empty event buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Returns `true` if there are no events.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
