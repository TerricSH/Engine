use std::sync::{Arc, Mutex};

use glam::Mat4;

use engine_renderer::debug_draw::{DebugDrawBuffer, DebugDrawProvider};

use crate::ColliderShape;

/// Debug visualisation data for a single collider.
#[derive(Clone, Debug)]
pub struct ColliderDebugInfo {
    pub shape: ColliderShape,
    pub position: glam::Vec3,
    pub rotation: glam::Quat,
}

/// Debug draw provider for physics colliders.
///
/// Registers with Gate 9's `DebugDrawRegistry` to render wireframe
/// collider shapes in the debug view.
pub struct PhysicsDebugDraw {
    /// Shared collider snapshot updated by `PhysicsWorld` after each step.
    colliders: Arc<Mutex<Vec<ColliderDebugInfo>>>,
}

impl PhysicsDebugDraw {
    /// Create a new `PhysicsDebugDraw` with an empty collider list.
    pub fn new() -> Self {
        Self {
            colliders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a `PhysicsDebugDraw` that shares collider data with the
    /// physics world.
    pub fn with_shared_data(colliders: Arc<Mutex<Vec<ColliderDebugInfo>>>) -> Self {
        Self { colliders }
    }

    /// Return a clone of the shared `Arc` so the physics world can push
    /// updates.
    pub fn shared_data(&self) -> Arc<Mutex<Vec<ColliderDebugInfo>>> {
        self.colliders.clone()
    }
}

impl Default for PhysicsDebugDraw {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugDrawProvider for PhysicsDebugDraw {
    fn name(&self) -> &str {
        "PhysicsDebugDraw"
    }

    fn populate(&self, buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {
        let colliders = match self.colliders.lock() {
            Ok(c) => c.clone(),
            Err(_) => return,
        };

        for info in &colliders {
            let color = [0.0, 1.0, 0.0, 1.0]; // green wireframe
            match &info.shape {
                ColliderShape::Cuboid { hx, hy, hz } => {
                    buffer.box_wireframe(info.position, glam::Vec3::new(*hx, *hy, *hz), color);
                }
                ColliderShape::Ball { radius } => {
                    buffer.sphere_wireframe(info.position, *radius, color);
                }
                ColliderShape::Capsule {
                    half_height,
                    radius,
                } => {
                    let top =
                        info.position + info.rotation * glam::Vec3::new(0.0, *half_height, 0.0);
                    let bottom =
                        info.position + info.rotation * glam::Vec3::new(0.0, -*half_height, 0.0);
                    buffer.sphere_wireframe(top, *radius, color);
                    buffer.sphere_wireframe(bottom, *radius, color);
                    buffer.line(top, bottom, color);
                }
            }
        }
    }
}
