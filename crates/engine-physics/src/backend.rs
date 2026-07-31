use std::collections::{HashMap, HashSet};

use crossbeam_channel::Sender;
use rapier3d::na;
use rapier3d::prelude::*;

use crate::components::{BodyType, Collider, ColliderShape, RigidBody};
use crate::convert::{from_rapier_isometry, from_rapier_vec, to_rapier_isometry, to_rapier_vec};
use crate::events::{
    CollisionEvent, CollisionEventKind, JointBreakEvent, PhysicsEvents, TriggerEvent,
    TriggerEventKind,
};
use crate::joints::{JointDescriptor, JointHandle, JointType};
use crate::queries::{
    OverlapHitResult, PhysicsQueryFilter, QueryBatcher, QueryResults, RaycastHitResult,
    SweepHitResult,
};
use crate::Entity;
use crate::RigidBodyRuntimeState;
use crate::Transform;

// Note: all rapier types are imported via rapier3d::prelude::* above.

// ── Raycast result ──────────────────────────────────────────────────────────

/// Result of a raycast query.
#[derive(Debug, Clone)]
pub struct RaycastHit {
    /// The entity that was hit.
    pub entity: Entity,
    /// World-space intersection point.
    pub point: glam::Vec3,
    /// Surface normal at the intersection point.
    pub normal: glam::Vec3,
    /// Distance from the ray origin to the intersection.
    pub distance: f32,
}

// ── Helper: convert ColliderShape to Rapier SharedShape ─────────────────────

pub(crate) fn to_rapier_shared_shape(shape: &ColliderShape) -> Option<SharedShape> {
    match shape {
        ColliderShape::Cuboid { hx, hy, hz }
            if [hx, hy, hz]
                .into_iter()
                .all(|value| value.is_finite() && *value > 0.0) =>
        {
            Some(SharedShape::cuboid(*hx, *hy, *hz))
        }
        ColliderShape::Ball { radius } if radius.is_finite() && *radius > 0.0 => {
            Some(SharedShape::ball(*radius))
        }
        ColliderShape::Capsule {
            half_height,
            radius,
        } if half_height.is_finite()
            && *half_height >= 0.0
            && radius.is_finite()
            && *radius > 0.0 =>
        {
            let a = na::Point3::new(0.0, -*half_height, 0.0);
            let b = na::Point3::new(0.0, *half_height, 0.0);
            Some(SharedShape::capsule(a, b, *radius))
        }
        ColliderShape::HeightField {
            rows,
            columns,
            heights,
            scale,
        } => {
            let valid = *rows >= 2
                && *columns >= 2
                && heights.len() == *rows as usize * *columns as usize
                && heights.iter().all(|height| height.is_finite())
                && scale.iter().all(|value| value.is_finite() && *value > 0.0);
            if !valid {
                // Collider data can arrive through generic script writes. A
                // malformed payload must not reach Rapier's assert-heavy
                // constructor or turn into a phantom fallback collider.
                return None;
            }
            let matrix = na::DMatrix::from_row_slice(*rows as usize, *columns as usize, heights);
            Some(SharedShape::heightfield(
                matrix,
                na::Vector3::new(scale[0], scale[1], scale[2]),
            ))
        }
        ColliderShape::TriMesh { vertices, indices } => {
            let valid = vertices.len() >= 3
                && !indices.is_empty()
                && vertices
                    .iter()
                    .flatten()
                    .all(|component| component.is_finite())
                && indices
                    .iter()
                    .flatten()
                    .all(|index| (*index as usize) < vertices.len());
            if !valid {
                return None;
            }
            let vertices = vertices
                .iter()
                .map(|position| na::Point3::new(position[0], position[1], position[2]))
                .collect();
            Some(SharedShape::trimesh(vertices, indices.clone()))
        }
        _ => None,
    }
}

type EntityPair = (Entity, Entity);

fn canonical_entity_pair(a: Entity, b: Entity) -> EntityPair {
    let a_key = (a.index(), a.generation());
    let b_key = (b.index(), b.generation());
    if a_key <= b_key {
        (a, b)
    } else {
        (b, a)
    }
}

// ── Internal event handler ──────────────────────────────────────────────────

/// Internal data for a collision (non-trigger) event.
#[derive(Debug, Clone)]
struct RawContactEvent {
    collider1: ColliderHandle,
    collider2: ColliderHandle,
    started: bool,
}

/// Internal data for a trigger / sensor intersection event.
#[derive(Debug, Clone)]
struct RawIntersectionEvent {
    collider1: ColliderHandle,
    collider2: ColliderHandle,
    intersecting: bool,
}

struct BackendEventHandler {
    tx_col: Sender<RawContactEvent>,
    tx_int: Sender<RawIntersectionEvent>,
}

impl EventHandler for BackendEventHandler {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        colliders: &ColliderSet,
        event: rapier3d::geometry::CollisionEvent,
        _contact_pair: Option<&ContactPair>,
    ) {
        let c1 = event.collider1();
        let c2 = event.collider2();

        // Route sensor (trigger) events through the intersection channel;
        // regular collisions go through the collision channel.
        let is_sensor = colliders.get(c1).map(|c| c.is_sensor()).unwrap_or(false)
            || colliders.get(c2).map(|c| c.is_sensor()).unwrap_or(false);

        if is_sensor {
            let _ = self.tx_int.send(RawIntersectionEvent {
                collider1: c1,
                collider2: c2,
                intersecting: event.started(),
            });
        } else {
            let _ = self.tx_col.send(RawContactEvent {
                collider1: c1,
                collider2: c2,
                started: event.started(),
            });
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &ContactPair,
        _total_force_magnitude: f32,
    ) {
        // Not forwarded in this implementation.
    }
}

// ── RapierBackend ───────────────────────────────────────────────────────────

/// Rapier 3D physics backend adapter.
///
/// Owns all Rapier simulation state and maintains maps from complete entity
/// handles (index plus generation) to Rapier handles. Backend handles are
/// NEVER serialised or exposed in
/// ECS components — they are purely internal to this adapter.
pub struct RapierBackend {
    pub(crate) pipeline: PhysicsPipeline,
    pub(crate) gravity: na::Vector3<f32>,
    pub(crate) integration: IntegrationParameters,
    pub(crate) islands: IslandManager,
    pub(crate) broad_phase: BroadPhaseMultiSap,
    pub(crate) narrow_phase: NarrowPhase,
    pub(crate) bodies: RigidBodySet,
    pub(crate) colliders: ColliderSet,
    pub(crate) impulse_joints: ImpulseJointSet,
    pub(crate) multibody_joints: MultibodyJointSet,
    pub(crate) ccd_solver: CCDSolver,
    pub(crate) query_pipeline: QueryPipeline,
    query_pipeline_dirty: bool,

    /// Maps complete entity handles to Rapier rigid body handles.
    pub(crate) body_map: HashMap<Entity, RigidBodyHandle>,
    /// Maps complete entity handles to Rapier collider handles and shapes.
    pub(crate) collider_map: HashMap<Entity, (ColliderHandle, ColliderShape)>,

    /// Maps complete entity handles to all attached joint handles.
    pub(crate) joint_entity_map: HashMap<Entity, HashSet<u32>>,
    /// Maps our JointHandle.0 → full Rapier ImpulseJointHandle (with generation).
    pub(crate) joint_handle_lookup: HashMap<u32, ImpulseJointHandle>,
    /// Authored break thresholds keyed by engine joint handle.
    joint_break_limits: HashMap<u32, (f32, f32)>,
    /// Complete body entities for break-event reporting.
    joint_bodies: HashMap<u32, (Entity, Entity)>,
    /// Auto-incrementing counter for JointHandle IDs.
    next_joint_id: u32,

    /// Active sensor (trigger) overlaps from the previous frame.
    /// Used to derive Entered vs Stay [`TriggerEvent`]s.
    /// Keys are canonical pairs of complete entity handles.
    active_sensor_overlaps: HashSet<EntityPair>,

    /// Active non‑sensor collision pairs from the previous frame.
    /// Used to derive [`CollisionEventKind::ContactStaying`].
    /// Keys are canonical pairs of complete entity handles.
    active_collision_overlaps: HashSet<EntityPair>,
}

mod bodies;
mod joints;
mod queries;
mod simulation;
mod state;

#[cfg(test)]
#[test]
fn production_facade_uses_real_modules() {
    let source = include_str!("backend.rs");
    assert!(!source.contains(concat!("include", "!(")));
    for module in ["bodies", "joints", "queries", "simulation", "state"] {
        assert!(source.contains(&format!("mod {module};")));
    }
}
