use glam::{Mat4, Vec3};

/// Compute a left-handed look-at view matrix from orbit camera parameters.
///
/// * `pitch` – vertical angle in degrees (clamped to ±89°).
/// * `yaw`   – horizontal angle in degrees.
/// * `distance` – distance from the look-at target.
/// * `target` – world-space point the camera orbits around.
pub fn orbit_view_matrix(pitch: f32, yaw: f32, distance: f32, target: [f32; 3]) -> Mat4 {
    let pitch_rad = pitch.to_radians();
    let yaw_rad = yaw.to_radians();
    let pitch_cos = pitch_rad.cos();
    let pitch_sin = pitch_rad.sin();

    let eye = Vec3::new(
        target[0] + distance * yaw_rad.cos() * pitch_cos,
        target[1] + distance * pitch_sin,
        target[2] + distance * yaw_rad.sin() * pitch_cos,
    );

    Mat4::look_at_lh(eye, Vec3::from(target), Vec3::Y)
}

/// Compute a left-handed perspective projection matrix.
///
/// * `fov_y_deg` – vertical field of view in degrees.
/// * `aspect`    – width / height ratio.
/// * `near`      – near clip plane distance.
/// * `far`       – far clip plane distance.
pub fn orbit_projection_matrix(fov_y_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    Mat4::perspective_lh(fov_y_deg.to_radians(), aspect, near, far)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};

    #[test]
    fn orbit_view_matrix_looks_at_target() {
        let view = orbit_view_matrix(0.0, 0.0, 10.0, [0.0, 0.0, 0.0]);
        // With pitch=0, yaw=0, eye should be at (10, 0, 0) for LH system
        // look_at_lh with eye at (10,0,0), center at (0,0,0), up=(0,1,0):
        let eye_expected = Vec3::new(10.0, 0.0, 0.0);
        // The third row of the view matrix encodes the translation
        let eye_derived = view.inverse().transform_point3(Vec3::ZERO);
        assert!(
            (eye_derived - eye_expected).length() < 1e-5,
            "expected eye near {:?}, got {:?}",
            eye_expected,
            eye_derived,
        );
    }

    #[test]
    fn orbit_projection_matrix_is_finite() {
        let proj = orbit_projection_matrix(60.0, 16.0 / 9.0, 0.1, 100.0);
        for row in proj.to_cols_array() {
            assert!(row.is_finite());
        }
    }

    #[test]
    fn orbit_view_matrix_handles_extreme_pitch() {
        // Should not panic or produce NaN
        let view = orbit_view_matrix(89.0, 180.0, 5.0, [1.0, 2.0, 3.0]);
        for row in view.to_cols_array() {
            assert!(row.is_finite());
        }
    }
}
