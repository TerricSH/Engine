use crate::{Entity, JointHandle};

// ══════════════════════════════════════════════════════════════════════════════
// Collision Events (non‑trigger contacts with physical response)
// ══════════════════════════════════════════════════════════════════════════════

/// The kind of collision event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionEventKind {
    /// Two colliders started touching.
    ContactStarted,
    /// Two colliders are still touching (reported every frame while touching).
    ContactStaying,
    /// Two colliders stopped touching.
    ContactStopped,
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

// ══════════════════════════════════════════════════════════════════════════════
// Trigger Events (sensor / trigger volumes — no physical response)
// ══════════════════════════════════════════════════════════════════════════════

/// The kind of trigger (sensor) event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerEventKind {
    /// Two trigger colliders just started overlapping.
    Entered,
    /// Two trigger colliders are still overlapping (reported every frame).
    Stay,
    /// Two trigger colliders stopped overlapping.
    Exited,
}

/// A single trigger event produced during a physics step.
#[derive(Clone, Debug)]
pub struct TriggerEvent {
    /// What kind of trigger event this is.
    pub kind: TriggerEventKind,
    /// The first entity involved.
    pub entity_a: Entity,
    /// The second entity involved.
    pub entity_b: Entity,
}

/// A joint that exceeded one of its authored break thresholds.
#[derive(Clone, Debug)]
pub struct JointBreakEvent {
    pub handle: JointHandle,
    /// Persistent constraint entity for an ECS-authored joint. Raw runtime
    /// joints that were not created from a component report `None`.
    pub joint_entity: Option<Entity>,
    pub entity_a: Entity,
    pub entity_b: Entity,
    /// Estimated constraint force in newtons for the fixed step that broke it.
    pub force: f32,
    /// Estimated constraint torque in newton-metres.
    pub torque: f32,
}

// ══════════════════════════════════════════════════════════════════════════════
// Event containers
// ══════════════════════════════════════════════════════════════════════════════

/// Collection of all events produced during one or more physics steps.
#[derive(Clone, Debug, Default)]
pub struct PhysicsEvents {
    /// Non‑trigger collision events (ContactStarted / ContactStopped).
    pub collisions: Vec<CollisionEvent>,
    /// Trigger (sensor) overlap events (Entered / Stay / Exited).
    pub triggers: Vec<TriggerEvent>,
    /// Constraint break events. The joint has already been removed.
    pub joint_breaks: Vec<JointBreakEvent>,
}

impl PhysicsEvents {
    /// Create a new empty event buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all events (both collisions and triggers).
    pub fn clear(&mut self) {
        self.collisions.clear();
        self.triggers.clear();
        self.joint_breaks.clear();
    }

    /// Returns `true` if there are no events of any kind.
    pub fn is_empty(&self) -> bool {
        self.collisions.is_empty() && self.triggers.is_empty() && self.joint_breaks.is_empty()
    }

    /// Returns the number of collision events.
    pub fn collision_count(&self) -> usize {
        self.collisions.len()
    }

    /// Returns the number of trigger events.
    pub fn trigger_count(&self) -> usize {
        self.triggers.len()
    }

    pub fn joint_break_count(&self) -> usize {
        self.joint_breaks.len()
    }
}
