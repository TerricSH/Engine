use rapier3d::na;

// ── Public coordinate conversion helpers ────────────────────────────────────

/// Convert a `glam::Vec3` to a Rapier `na::Vector3<f32>`.
#[inline]
pub fn to_rapier_vec(v: glam::Vec3) -> na::Vector3<f32> {
    na::Vector3::new(v.x, v.y, v.z)
}

/// Convert a Rapier `na::Vector3<f32>` to a `glam::Vec3`.
#[inline]
pub fn from_rapier_vec(v: na::Vector3<f32>) -> glam::Vec3 {
    glam::Vec3::new(v.x, v.y, v.z)
}

// ── Private conversion helpers ──────────────────────────────────────────────

#[inline]
pub(crate) fn to_rapier_isometry(translation: glam::Vec3, rotation: glam::Quat) -> na::Isometry3<f32> {
    na::Isometry3::from_parts(
        na::Translation3::new(translation.x, translation.y, translation.z),
        na::UnitQuaternion::from_quaternion(na::Quaternion::new(
            rotation.w, rotation.x, rotation.y, rotation.z,
        )),
    )
}

#[inline]
pub(crate) fn from_rapier_isometry(pos: &na::Isometry3<f32>) -> (glam::Vec3, glam::Quat) {
    let t = pos.translation.vector;
    let r = pos.rotation.quaternion();
    (
        glam::Vec3::new(t.x, t.y, t.z),
        glam::Quat::from_xyzw(r.i, r.j, r.k, r.w),
    )
}
