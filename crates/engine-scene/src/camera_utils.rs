//! Camera utility helpers for common view setups.
//!
//! These functions compute view matrices for common camera placements.
//! They are designed to work with the built-in [`Camera`] and [`Transform`]
//! components during the extraction phase.

use glam::{Mat4, Quat, Vec3};

/// Build a view matrix for a third-person camera that orbits around a target.
///
/// The camera is positioned at `target + offset` and always looks at `target`.
pub fn orbit_view_matrix(target: Vec3, offset: Vec3) -> Mat4 {
    let eye = target + offset;
    Mat4::look_at_rh(eye, target, Vec3::Y)
}

/// Build a view matrix for a third-person camera with explicit spherical
/// coordinates around a target.
///
/// * `target` — the position the camera looks at.
/// * `distance` — radial distance from the target.
/// * `height` — vertical offset from the target.
/// * `yaw` — horizontal rotation in radians.
pub fn spherical_orbit_view(target: Vec3, distance: f32, height: f32, yaw: f32) -> Mat4 {
    let eye = target
        + Vec3::new(yaw.sin() * distance, height, yaw.cos() * distance);
    Mat4::look_at_rh(eye, target, Vec3::Y)
}

/// Build a first-person view matrix from a position and look direction.
pub fn first_person_view(position: Vec3, forward: Vec3, up: Vec3) -> Mat4 {
    Mat4::look_at_rh(position, position + forward, up)
}

/// Compute the translation and rotation for an orbit camera entity
/// so it looks at `target` from `offset` (e.g. `(0, 5, 8)`).
pub fn setup_orbit_transform(target: Vec3, offset: Vec3) -> (Vec3, Quat) {
    let eye = target + offset;
    let dir_to_target = (target - eye).normalize();
    let rotation = Quat::from_rotation_arc(-Vec3::Z, dir_to_target);
    (eye, rotation)
}
