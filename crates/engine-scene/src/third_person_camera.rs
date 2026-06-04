//! Third-person camera component and follow system.
//!
//! Provides a [`ThirdPersonCamera`] component that makes a camera entity
//! follow a target entity with a configurable offset and look-at behaviour.
//! Call [`update_follow`] each frame to sync the camera's [`Transform`].

use glam::{Quat, Vec3};

use crate::components::Transform;
use crate::World;

/// Configuration for a third-person camera that follows a target entity.
#[derive(Clone, Debug)]
pub struct ThirdPersonCamera {
    /// Entity index of the target to follow.
    pub target_entity: crate::Entity,
    /// World-space offset from the target (e.g. `(0.0, 5.0, 8.0)`).
    pub offset: Vec3,
    /// Damping factor for smooth follow (0 = instant, 1 = no movement).
    pub damping: f32,
}

impl ThirdPersonCamera {
    const DEFAULT_OFFSET: Vec3 = Vec3::new(0.0, 5.0, 8.0);

    /// Create a new camera config following the given entity.
    pub fn new(target_entity: crate::Entity) -> Self {
        Self {
            target_entity,
            offset: Self::DEFAULT_OFFSET,
            damping: 0.0,
        }
    }

    /// Update the camera entity's Transform so it looks at the target.
    ///
    /// Call this each frame before extraction to ensure the view matrix
    /// reflects the current camera position.
    ///
    /// Returns `true` if the camera was updated, `false` if the target or
    /// camera entity could not be found in the world.
    pub fn update(&self, world: &mut World, camera_transform: &mut Transform) -> bool {
        // Read the target's position.
        let target_pos = match world.get::<Transform>(self.target_entity) {
            Some(t) => t.translation,
            None => return false,
        };

        // Compute desired eye position.
        let desired_eye = target_pos + self.offset;

        // Apply damping (lerp between current and desired position).
        let current_eye = camera_transform.translation;
        let eye = if self.damping > 0.0 {
            current_eye.lerp(desired_eye, 1.0 - self.damping)
        } else {
            desired_eye
        };

        // Compute rotation so -Z (forward) points at the target.
        let dir = (target_pos - eye).normalize_or_zero();
        let rotation = if dir.length_squared() > 0.001 {
            Quat::from_rotation_arc(-Vec3::Z, dir)
        } else {
            camera_transform.rotation
        };

        camera_transform.translation = eye;
        camera_transform.rotation = rotation;
        true
    }

    /// Convenience: find the camera entity and update it in one call.
    ///
    /// Looks up the camera entity by index and updates its Transform.
    pub fn apply(&self, world: &mut World, camera_entity: crate::Entity) -> bool {
        let target_pos = match world.get::<Transform>(self.target_entity) {
            Some(t) => t.translation,
            None => return false,
        };

        if let Some(ct) = world.get_mut::<Transform>(camera_entity) {
            let desired_eye = target_pos + self.offset;
            let eye = if self.damping > 0.0 {
                ct.translation.lerp(desired_eye, 1.0 - self.damping)
            } else {
                desired_eye
            };
            let dir = (target_pos - eye).normalize_or_zero();
            let rotation = if dir.length_squared() > 0.001 {
                Quat::from_rotation_arc(-Vec3::Z, dir)
            } else {
                ct.rotation
            };
            ct.translation = eye;
            ct.rotation = rotation;
            true
        } else {
            false
        }
    }
}
