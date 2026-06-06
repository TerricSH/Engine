//! PlayerPawn — one-shot assembly of a playable character.
//!
//! [`create_player_pawn`] creates a complete third-person walking pawn in the
//! ECS world: a ground plane, a player capsule, and a follow camera.

use engine_physics::{BodyType, Collider, ColliderShape, RigidBody};
use engine_scene::components::{Camera, Renderable, Transform};
use engine_scene::third_person_camera::ThirdPersonCamera;
use engine_scene::{Entity, World};
use glam::Vec3;

use crate::CharacterController;

/// Result of assembling a player pawn into a [`World`].
pub struct PlayerPawn {
    /// Ground plane entity (static rigid-body + collider).
    pub ground: Entity,
    /// Player capsule entity (character controller + renderable).
    pub player: Entity,
    /// Follow-camera entity.
    pub camera: Entity,
    /// Camera controller config (call `.apply(world, camera)` each frame).
    pub camera_controller: ThirdPersonCamera,
    /// Kinematic character controller (call `.update()` each frame).
    pub controller: CharacterController,
    /// Mesh ID used for the ground plane (for upload_mesh).
    pub ground_mesh_id: String,
    /// Mesh ID used for the player (for upload_mesh).
    pub player_mesh_id: String,
}

/// Create a complete third-person walking pawn in the given `World`.
///
/// After calling this:
/// * Upload a coloured-quad mesh for `result.ground_mesh_id`
///   and a coloured-capsule mesh for `result.player_mesh_id`.
/// * Each frame, call `result.camera_controller.apply(world, result.camera)`
///   before `render_frame()`.
/// * Each frame, call `result.controller.update(&input, Some(physics))`
///   and write `result.controller.position()` back to the player's Transform.
pub fn create_player_pawn(world: &mut World) -> PlayerPawn {
    // ── Ground (static rigid-body + collider) ─────────────────────────
    let ground = world.create_entity();
    world.add_component(
        ground,
        Transform {
            translation: Vec3::new(0.0, -0.5, 0.0),
            ..Transform::default()
        },
    );
    world.add_component(
        ground,
        RigidBody {
            body_type: BodyType::Static,
            ..RigidBody::default()
        },
    );
    world.add_component(
        ground,
        Collider {
            shape: ColliderShape::Cuboid {
                hx: 10.0,
                hy: 0.5,
                hz: 10.0,
            },
            ..Collider::default()
        },
    );
    world.add_component(
        ground,
        Renderable {
            mesh_asset: "mesh-ground".into(),
            material_asset: "default".into(),
            visible: true,
            cast_shadows: false,
            render_layer: "default".into(),
        },
    );

    // ── Player capsule (kinematic character, no rigid-body) ───────────
    let player = world.create_entity();
    world.add_component(
        player,
        Transform {
            translation: Vec3::new(0.0, 3.0, 0.0),
            ..Transform::default()
        },
    );
    // Keep the collider for ray-cast queries by the character controller.
    world.add_component(
        player,
        Collider {
            shape: ColliderShape::Capsule {
                half_height: 0.75,
                radius: 0.3,
            },
            ..Collider::default()
        },
    );
    world.add_component(
        player,
        Renderable {
            mesh_asset: "mesh-hero".into(),
            material_asset: "default".into(),
            visible: true,
            cast_shadows: true,
            render_layer: "default".into(),
        },
    );

    // ── Follow camera ─────────────────────────────────────────────────
    let camera = world.create_entity();
    world.add_component(
        camera,
        Transform {
            translation: Vec3::new(0.0, 5.0, 8.0),
            ..Transform::default()
        },
    );
    world.add_component(camera, Camera::default());

    let camera_controller = ThirdPersonCamera::new(player);
    let controller = CharacterController::new();

    PlayerPawn {
        ground,
        player,
        camera,
        camera_controller,
        controller,
        ground_mesh_id: "mesh-ground".to_string(),
        player_mesh_id: "mesh-hero".to_string(),
    }
}
