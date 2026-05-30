use serde::{Deserialize, Serialize};

use engine_scene::Component;

// ── BodyType ────────────────────────────────────────────────────────────────

/// Determines how a rigid body participates in the physics simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    /// Immovable body with infinite mass.
    Static,
    /// Fully simulated body affected by forces and collisions.
    Dynamic,
    /// Body moved by user-controlled velocity; not affected by forces.
    Kinematic,
}

// ── RigidBody ───────────────────────────────────────────────────────────────

/// Physics rigid body component.
///
/// Serialisable — does NOT contain backend handles.
/// Backend handles are managed internally by `RapierBackend`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RigidBody {
    /// Whether the body is static, dynamic, or kinematic.
    pub body_type: BodyType,
    /// Mass of the body in kilograms (only used for dynamic bodies).
    pub mass: f32,
    /// Linear damping factor (0 = no damping).
    pub linear_damping: f32,
    /// Angular damping factor (0 = no damping).
    pub angular_damping: f32,
    /// Whether the body participates in simulation.
    pub enabled: bool,
    /// Multiplier applied to gravity for this body.
    pub gravity_scale: f32,
    /// Whether the body can go to sleep when idle.
    pub can_sleep: bool,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: BodyType::Dynamic,
            mass: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            enabled: true,
            gravity_scale: 1.0,
            can_sleep: true,
        }
    }
}

impl Component for RigidBody {
    const TYPE_ID: &'static str = "engine.physics.rigid_body";
}

// ── ColliderShape ───────────────────────────────────────────────────────────

/// Shape of a collider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// Axis-aligned box defined by half-extents.
    Cuboid { hx: f32, hy: f32, hz: f32 },
    /// Sphere with the given radius.
    Ball { radius: f32 },
    /// Capsule (cylinder with hemispherical caps) aligned to local +Y.
    Capsule { half_height: f32, radius: f32 },
}

// ── Collider ────────────────────────────────────────────────────────────────

/// Physics collider component.
///
/// Serialisable — does NOT contain backend handles.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Collider {
    /// The geometric shape of the collider.
    pub shape: ColliderShape,
    /// Density in kg/m³ (used to compute mass from volume).
    pub density: f32,
    /// Coulomb friction coefficient (0 = frictionless).
    pub friction: f32,
    /// Restitution (bounciness) coefficient (0 = inelastic, 1 = perfectly elastic).
    pub restitution: f32,
    /// If true, the collider acts as a trigger (no physical response).
    pub is_trigger: bool,
    /// The collision group this collider belongs to.
    pub collision_group: u32,
    /// Bitmask of groups this collider collides with.
    pub collision_mask: u32,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Cuboid {
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
            },
            density: 1.0,
            friction: 0.5,
            restitution: 0.0,
            is_trigger: false,
            collision_group: 0xFFFF_FFFF,
            collision_mask: 0xFFFF_FFFF,
        }
    }
}

impl Component for Collider {
    const TYPE_ID: &'static str = "engine.physics.collider";
}

// ── PhysicsMaterial ─────────────────────────────────────────────────────────

/// Override material properties for a collider.
///
/// When attached to an entity alongside a [`Collider`], these values
/// override the collider's default friction / restitution / density.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhysicsMaterial {
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
}

impl Default for PhysicsMaterial {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
            density: 1.0,
        }
    }
}

impl Component for PhysicsMaterial {
    const TYPE_ID: &'static str = "engine.physics.physics_material";
}
